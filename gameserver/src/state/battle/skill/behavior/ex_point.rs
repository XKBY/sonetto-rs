//! ExPoint action — handler for the three behavior variants that
//! mutate per-entity ExPoint (Moxie / Faith) state:
//!
//! * `AddExPoint { amount }` — `20002` family. Emit a moxie_change of
//!   `amount` on the target.
//! * `AddExPointWithMax { amount }` — same payload, distinguished only
//!   so the dispatcher can tag it (no behavioral diff today).
//! * `ConsumeExPointAddAttr { min_consume, max_consume }` — behavior
//!   `60174`. Pulls the most recent ExPoint decrement on the caster,
//!   clamps to `[min, max]`, and converts that to a skill rate bonus
//!   using `rate_per_point` parsed from the `60174#attr#rate#min#max`
//!   config encoding. No emission — the bonus is applied via
//!   `executor.add_skill_rate_bonus`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::skill::cache::resolve_skill_effect_id;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// ExPoint action — routes from the three ExPoint-mutating variants.
pub(super) struct ExPoint;

impl BehaviorAction for ExPoint {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::AddExPoint { amount } | BehaviorType::AddExPointWithMax { amount } => {
                Some(Ok(vec![ActEffectBuilder::moxie_change(
                    ctx.target, *amount,
                )]))
            }
            BehaviorType::ConsumeExPointAddAttr {
                min_consume,
                max_consume,
            } => Some(execute_consume_ex_point_add_attr(
                ctx,
                *min_consume,
                *max_consume,
            )),
            _ => None,
        }
    }
}

fn execute_consume_ex_point_add_attr(
    ctx: &mut ActionCtx<'_, '_>,
    min_consume: i32,
    max_consume: i32,
) -> Result<Vec<ActEffect>> {
    let consumed = ctx
        .managers
        .ex_point_mgr
        .get_recent_decr_ex_point(ctx.caster_uid)
        .max(0);
    let usable = consumed.clamp(min_consume, max_consume);
    if usable <= 0 {
        return Ok(vec![]);
    }
    // Encoding: 60174#attr_id#rate_per_point#min#max...
    // The parser keeps min/max; pull rate_per_point from the config behavior string.
    let cfg = config::configs::get();
    let skill_effect_id = resolve_skill_effect_id(ctx.skill_id);
    let mut rate_per_point = 0;
    if let Some(skill_row) = cfg.skill_effect.iter().find(|s| s.id == skill_effect_id) {
        let raw_behaviors = [
            &skill_row.behavior1,
            &skill_row.behavior2,
            &skill_row.behavior3,
            &skill_row.behavior4,
            &skill_row.behavior5,
            &skill_row.behavior6,
            &skill_row.behavior7,
            &skill_row.behavior8,
            &skill_row.behavior9,
            &skill_row.behavior10,
        ];
        for raw in raw_behaviors {
            if raw.starts_with("60174#") {
                rate_per_point = raw
                    .split('#')
                    .nth(2)
                    .and_then(|v| v.parse::<i32>().ok())
                    .unwrap_or(0);
                if rate_per_point != 0 {
                    break;
                }
            }
        }
    }
    if rate_per_point == 0 {
        return Ok(vec![]);
    }
    ctx.executor.add_skill_rate_bonus(
        ctx.caster_uid,
        ctx.caster_uid,
        rate_per_point.saturating_mul(usable),
    );
    // Live payload does not emit an extra ATTR(26) step for this behavior.
    Ok(vec![])
}
