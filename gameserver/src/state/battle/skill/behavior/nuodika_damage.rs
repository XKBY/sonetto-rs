//! NuoDiKaDamage action — Nautika's Dual Faith damage variant.
//!
//! The variant carries `(primary_buff_id, primary_rate,
//! secondary_buff_id, secondary_rate, self_loss_param)`. The buff
//! ids are looked up in `buff_actions::attr_replace` to produce
//! per-buff permille values; the two are combined with their rates
//! into a `total_permille` of caster max HP. The caster takes
//! `current_hp × (self_loss_param/5) %` self-damage, then targets
//! resolved through `TargetResolver` each take
//! `max_hp × total_permille / 1000`.

use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::super::cache::resolve_skill_effect_id;
use crate::state::battle::buff_actions::attr_replace::buff_get_attr_replace_permille;
use crate::state::battle::skill::targets::{TargetResolver, get_entity};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::utils::damage_with_hurt;

pub(super) struct NuoDiKaDamage;

impl BehaviorAction for NuoDiKaDamage {
    fn execute(
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        let BehaviorType::NuoDiKaDamage {
            primary_buff_id,
            primary_rate,
            secondary_buff_id,
            secondary_rate,
            self_loss_param,
        } = behavior
        else {
            return Ok(vec![]);
        };

        let fight = ctx.behavior_ctx.fight;
        let Some(caster) = get_entity(fight, ctx.caster_uid) else {
            return Ok(vec![]);
        };
        let current_hp = caster.current_hp.unwrap_or(0);
        let max_hp = caster
            .attr
            .as_ref()
            .and_then(|a| a.hp)
            .unwrap_or(current_hp)
            .max(current_hp)
            .max(0);
        let primary_permille = buff_get_attr_replace_permille(*primary_buff_id).unwrap_or(0);
        let secondary_permille = buff_get_attr_replace_permille(*secondary_buff_id).unwrap_or(0);
        let total_permille = (primary_permille.saturating_mul(*primary_rate) / 1000)
            .saturating_add(secondary_permille.saturating_mul(*secondary_rate) / 1000)
            .max(0);
        let self_loss_percent = (*self_loss_param / 5).max(0);
        let self_loss = current_hp.saturating_mul(self_loss_percent) / 100;

        let cfg = config::configs::get();
        let logic_target = cfg
            .skill_effect
            .iter()
            .find(|s| s.id == resolve_skill_effect_id(ctx.skill_id))
            .and_then(|s| s.logic_target.trim().parse::<i32>().ok())
            .unwrap_or(0);
        let damage_targets = TargetResolver::new(fight, ctx.caster_uid, ctx.target)
            .behavior(logic_target)
            .resolve();

        let mut out = Vec::new();
        if self_loss > 0 {
            out.push(damage_with_hurt(
                ctx.caster_uid,
                self_loss,
                30006,
                ctx.skill_id,
                ctx.caster_uid,
            ));
        }
        if total_permille <= 0 {
            return Ok(out);
        }
        let damage = (max_hp.saturating_mul(total_permille) / 1000).max(1);
        for damage_target in damage_targets {
            if damage_target == 0 || damage_target == ctx.caster_uid {
                continue;
            }
            out.push(damage_with_hurt(
                damage_target,
                damage,
                -1,
                ctx.skill_id,
                ctx.caster_uid,
            ));
        }
        Ok(out)
    }
}
