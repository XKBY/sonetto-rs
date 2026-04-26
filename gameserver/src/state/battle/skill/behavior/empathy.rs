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

pub(super) struct Empathy;

impl BehaviorAction for Empathy {
    fn execute(
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
