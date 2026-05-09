//! `SettleDotAndCostDotDuration` (skill_behavior id 60073) — round-start
//! Poison-settle on the carrier. Tuesday's `30980151` is the only fixture
//! caller today, delivered to enemies via `magic_circle 22100003.enemy_skills`.
//!
//! Per the in-game text on `30980151`: "At the start of the round, resolve 1
//! round of [Poison] effects." The carrier walks its own Poison /
//! DeadlyPoison buffs and emits a single consolidated Genesis-crit damage
//! sum equal to `Σ (poisoner.atk × permille / 1000 × stacks) × 1.39` across
//! all such buffs. The same crit-hybrid multiplier (`1.39`) the regular
//! round-end DOT settlement uses applies here so the kill thresholds line
//! up — see `mechanics/dot.rs` module doc.
//!
//! Empirical LIVE shape (battle3 r5–r9, all `30980151` emissions):
//!
//! ```text
//! actType=SKILL actId=30980151 fromId=carrier toId=carrier
//!   actEffect:
//!     effectType=131 (OriginCrit) effectNum=Σdamage targetId=carrier configEffect=60073
//!     effectType=9   (Dead)        effectNum=0      targetId=carrier  // only if carrier dies
//! ```
//!
//! ## Why no duration decrement here
//!
//! The behavior name suggests it consumes duration ("CostDotDuration"), but
//! Tuesday's array `22100003` always pairs `30980151` with the lock-duration
//! debuff `30980131` (features `"810"`, `LockPoison`). The lock pins
//! `duringTime` so the same Poison stacks keep ticking damage every round
//! the array is active — "if tick is 2 after 3 rounds it still be 2 not 0".
//!
//! For non-Tuesday-array carriers the standard `BuffMgr::on_round_end`
//! handles duration decrement. So this handler doesn't decrement at all —
//! the regular tick path is the source of truth for buff lifetime, and the
//! lock prevents that path from firing on locked carriers.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::mechanics::dot::parse_dot_features;
use crate::state::battle::skill::get_entity;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::utils::apply_real_hurt_fix;

/// configEffect marker for 60073 emissions. Lets downstream observers
/// recognize "Poison settled by Tuesday's array" vs regular round-end
/// DOT (which has `configEffect = 0`).
const SETTLE_CONFIG_EFFECT: i32 = 60073;

/// Crit-hybrid multiplier — same value `mechanics/dot.rs` uses on its
/// round-end Poison ticks. LIVE emits these as `et=131 OriginCrit` with
/// damage ≈ 1.39× the un-crit value; we match.
const CRIT_PERMILLE: i32 = 1390;

pub(super) struct DotSettle;

impl BehaviorAction for DotSettle {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let _rounds = match behavior {
            BehaviorType::SettleDotAndCostDotDuration { rounds } => *rounds,
            _ => return None,
        };

        // The carrier is the entity executing the passive — for Tuesday's
        // array, this is each living enemy reached by the circle's
        // `enemy_skills` advertisement. The skill is self-targeted so
        // `target` and `caster_uid` agree.
        let carrier = ctx.caster_uid;
        if carrier == 0 {
            return Some(Ok(Vec::new()));
        }

        let buffs = ctx.managers.buff_mgr.get(carrier).to_vec();
        if buffs.is_empty() {
            return Some(Ok(Vec::new()));
        }

        let mut total_damage: i32 = 0;
        for instance in &buffs {
            let Some((_marker_et, permille)) = parse_dot_features(instance.buff_id) else {
                continue;
            };
            let Some(poisoner) = get_entity(ctx.behavior_ctx.fight, instance.from_uid) else {
                continue;
            };
            let poisoner_atk = poisoner.attr.as_ref().and_then(|a| a.attack).unwrap_or(0);
            if poisoner_atk <= 0 {
                continue;
            }
            let base = apply_real_hurt_fix(
                &ctx.managers.buff_mgr,
                carrier,
                poisoner_atk * permille / 1000,
            );
            if base <= 0 {
                continue;
            }
            let stacks = instance.layer.max(1);
            let crit_dmg = base.saturating_mul(CRIT_PERMILLE) / 1000;
            total_damage = total_damage.saturating_add(crit_dmg.saturating_mul(stacks));
        }

        if total_damage <= 0 {
            return Some(Ok(Vec::new()));
        }

        let mut effects = vec![
            ActEffectBuilder::origin_crit(carrier, total_damage, Some(SETTLE_CONFIG_EFFECT)),
        ];

        // Append `et=9 Dead` if the consolidated damage drops the carrier
        // below 0 HP — matches LIVE r5 step where enemy `-5` dies from a
        // single 7967 settle hit and the SKILL wrapper carries both packets.
        if let Some(victim) = get_entity(ctx.behavior_ctx.fight, carrier) {
            let hp = victim.current_hp.unwrap_or(0);
            let shield = victim.shield_value.unwrap_or(0);
            let after_shield = total_damage.saturating_sub(shield);
            if after_shield > 0 && hp - after_shield <= 0 {
                effects.push(
                    ActEffectBuilder::dead(carrier),
                );
            }
        }

        Some(Ok(effects))
    }
}
