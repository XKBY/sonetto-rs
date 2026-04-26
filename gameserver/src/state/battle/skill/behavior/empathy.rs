use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::{
    fight_step::ActEffectBuilder,
    mechanics::empathy::{EMPATHY_DEFAULT_BUFF_ID, EMPATHY_TYPE_ID, EmpathyState},
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
    utils::effect_none,
};

// Solace ranks 1/2/3 = skill_effect 30800121/22/23. The self-loss
// permille is config-driven from the `RealDamageSelfAndAddBuff` behavior
// param (`60039#permille#buff_id`), so Lv1=100‰ (10% MaxHP), Lv2=150‰
// (15%), Lv3=200‰ (20%) — see skill_effect.json behaviors:
//   30800121: behavior2 = 60039#100#30800111
//   30800122: behavior2 = 60039#150#30800111
//   30800123: behavior2 = 60039#200#30800111
const SOLACE_SKILL_IDS: [i32; 3] = [30800121, 30800122, 30800123];
const SOLACE_CONFIG_EFFECT: i32 = 60039;
/// Marker value attached to the Genesis bonus emission. The executor
/// fallback gate uses this to distinguish bonus damage (which should
/// not suppress the primary `damageRate` damage emission) from
/// primary-damage behavior emissions.
pub const SUBCONSCIOUS_BONUS_CONFIG_EFFECT: i32 = 60038;
/// Marker value attached to Kakania's EX consume-and-bonus emission.
/// Same role as `SUBCONSCIOUS_BONUS_CONFIG_EFFECT` for the executor's
/// fallback gate, plus broadcast on the `StorageInjury(167)` reset
/// emission so LIVE-side observers can tell the bank cleared.
pub const EX_CONSUME_CONFIG_EFFECT: i32 = 60040;

pub(super) struct Empathy;

impl BehaviorAction for Empathy {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::RealDamageSelfAndAddBuffToTarget { .. } => {
                self.execute_solace(behavior, ctx, condition)
            }
            BehaviorType::OriginDamageFromInjuryBank { multiplier_permille } => Some(Ok(self
                .execute_subconscious_bonus(ctx, *multiplier_permille))),
            BehaviorType::ConsumeInjuryBankAndDamage { multiplier_permille } => {
                Some(Ok(self.execute_ex_consume(ctx, *multiplier_permille)))
            }
            _ => None,
        }
    }
}

impl Empathy {
    fn execute_subconscious_bonus(
        &self,
        ctx: &mut ActionCtx<'_, '_>,
        multiplier_permille: i32,
    ) -> Vec<ActEffect> {
        // Bonus only fires when the caster has stored Empathy. The
        // mechanic state is the source of truth; its values are
        // (re)seeded each round from the Empathy buff's
        // `actCommonParams`, so `current` is up to date here.
        let current = ctx.mechanics.empathy.current(ctx.caster_uid);
        if current <= 0 || multiplier_permille <= 0 || ctx.target == 0 {
            return Vec::new();
        }
        let bonus = current.saturating_mul(multiplier_permille) / 1000;
        if bonus <= 0 {
            return Vec::new();
        }
        // Genesis DMG ignores defense — emit raw OriginDamage(130).
        // `config_effect = 60038` flags this as a bonus emission so the
        // executor's `has_damage_effect` gate still fires the primary
        // `damageRate` damage path.
        vec![
            ActEffectBuilder::new(EffectType::OriginDamage as i32, ctx.target)
                .effect_num(bonus)
                .config_effect(SUBCONSCIOUS_BONUS_CONFIG_EFFECT)
                .build(),
        ]
    }

    fn execute_ex_consume(
        &self,
        ctx: &mut ActionCtx<'_, '_>,
        multiplier_permille: i32,
    ) -> Vec<ActEffect> {
        // The EX behavior fires once per cast; if the caster has no
        // Empathy yet, we still emit nothing (LIVE only emits the
        // 167/130 pair when there is something to consume).
        let current = ctx.mechanics.empathy.current(ctx.caster_uid);
        if current <= 0 || multiplier_permille <= 0 || ctx.target == 0 {
            return Vec::new();
        }
        let bonus = current.saturating_mul(multiplier_permille) / 1000;

        // Caster max-HP needed by `sync_buff_state` to re-derive the
        // storage cap; same lookup the Solace path uses below.
        let max_hp = ctx
            .behavior_ctx
            .fight
            .attacker
            .as_ref()
            .into_iter()
            .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
            .chain(
                ctx.behavior_ctx
                    .fight
                    .defender
                    .as_ref()
                    .into_iter()
                    .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
            )
            .find(|entity| entity.uid == Some(ctx.caster_uid))
            .and_then(|entity| entity.attr.as_ref())
            .and_then(|attr| attr.hp)
            .unwrap_or(0);
        let cap = EmpathyState::storage_cap(max_hp);

        let (empathy_buff_id, buff_uid) = ctx
            .managers
            .buff_mgr
            .find_instance_by_type_id(ctx.caster_uid, EMPATHY_TYPE_ID)
            .map(|buff| (buff.buff_id, buff.uid))
            .unwrap_or((EMPATHY_DEFAULT_BUFF_ID, 0));

        // LIVE r8 step[2] for skill 30800131 emits, in order:
        //   et=167 StorageInjury cfx=60040 num=0 (Empathy reset
        //          broadcast targeting the caster)
        //   et=130 OriginDamage  cfx=60040 num=bonus ti=target
        // The reset is what flags the bank-clear to downstream
        // observers (heal-on-storage threshold listeners, etc.).
        let mut effects = Vec::new();
        effects.push(self.build_storage_injury_reset_marker(
            ctx.caster_uid,
            empathy_buff_id,
            buff_uid,
            cap,
        ));
        effects.push(
            ActEffectBuilder::new(EffectType::OriginDamage as i32, ctx.target)
                .effect_num(bonus)
                .config_effect(EX_CONSUME_CONFIG_EFFECT)
                .build(),
        );

        // Reset the mechanic state AFTER computing the bonus and the
        // reset marker, since the reset marker carries the new value
        // (0). `sync_buff_state` writes both the in-memory cache and
        // the buff's `actCommonParams` so subsequent behaviors see 0.
        ctx.mechanics
            .empathy
            .sync_buff_state(&mut ctx.managers.buff_mgr, ctx.caster_uid, 0, max_hp);

        effects
    }

    fn build_storage_injury_reset_marker(
        &self,
        caster_uid: i64,
        empathy_buff_id: i32,
        buff_uid: i64,
        cap: i32,
    ) -> ActEffect {
        use sonettobuf::BuffInfo;
        ActEffect {
            effect_type: Some(EffectType::StorageInjury as i32),
            target_id: Some(caster_uid),
            effect_num: Some(0),
            config_effect: Some(EX_CONSUME_CONFIG_EFFECT),
            buff: Some(BuffInfo {
                buff_id: Some(empathy_buff_id),
                duration: Some(0),
                uid: Some(buff_uid),
                ex_info: Some(0),
                from_uid: Some(caster_uid),
                count: Some(0),
                act_common_params: Some(format!("770#0#{}", cap.max(0))),
                layer: Some(0),
                r#type: Some(crate::state::battle::types::buff::BuffLayerType::Normal as i32),
                act_info: vec![],
            }),
            ..Default::default()
        }
    }

    fn execute_solace(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let BehaviorType::RealDamageSelfAndAddBuffToTarget {
            amount_permille,
            buff_id,
        } = behavior
        else {
            return None;
        };
        if !SOLACE_SKILL_IDS.contains(&ctx.skill_id) || ctx.target == ctx.caster_uid {
            return Some(Ok(vec![]));
        }

        let max_hp = ctx
            .behavior_ctx
            .fight
            .attacker
            .as_ref()
            .into_iter()
            .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
            .chain(
                ctx.behavior_ctx
                    .fight
                    .defender
                    .as_ref()
                    .into_iter()
                    .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
            )
            .find(|entity| entity.uid == Some(ctx.caster_uid))
            .and_then(|entity| entity.attr.as_ref())
            .and_then(|attr| attr.hp)
            .unwrap_or(0);
        if max_hp <= 0 {
            return Some(Ok(vec![]));
        }

        let self_damage = max_hp.saturating_mul(*amount_permille) / 1000;
        let storage_amount = EmpathyState::compute_storage_amount(self_damage);
        let cap = EmpathyState::storage_cap(max_hp);
        let current_total = ctx.mechanics.empathy.apply_storage(
            &mut ctx.managers.buff_mgr,
            ctx.caster_uid,
            storage_amount,
            max_hp,
        );
        // Look up by typeId so portrait/destiny variants
        // (30800142/30800143) match the same Empathy mechanic — see
        // `mechanics/empathy.rs::EMPATHY_TYPE_ID`. Falls back to the
        // canonical 30800141 when no instance exists yet.
        let (empathy_buff_id, buff_uid) = ctx
            .managers
            .buff_mgr
            .find_instance_by_type_id(ctx.caster_uid, EMPATHY_TYPE_ID)
            .map(|buff| (buff.buff_id, buff.uid))
            .unwrap_or((EMPATHY_DEFAULT_BUFF_ID, 0));

        let mut effects = vec![
            ctx.mechanics.empathy.emit_storage_injury(
                ctx.caster_uid,
                current_total,
                empathy_buff_id,
                buff_uid,
                ctx.caster_uid,
                cap,
            ),
            ctx.mechanics.empathy.emit_buff_update(
                ctx.caster_uid,
                current_total,
                empathy_buff_id,
                buff_uid,
                ctx.caster_uid,
                cap,
            ),
            ActEffectBuilder::new(EffectType::OriginDamage as i32, ctx.caster_uid)
                .effect_num(self_damage)
                .config_effect(SOLACE_CONFIG_EFFECT)
                .build(),
        ];
        effects.extend(buff::apply(
            ctx.executor,
            ctx.behavior_ctx.fight,
            ctx.managers,
            ctx.mechanics,
            ctx.caster_uid,
            ctx.target,
            *buff_id,
            0,
            ctx.mechanics.bloodtithe.has_bloodpool(),
            ctx.skill_id,
            ctx.condition_id,
            condition,
        ));
        effects.push(effect_none(ctx.target));

        Some(Ok(effects))
    }
}
