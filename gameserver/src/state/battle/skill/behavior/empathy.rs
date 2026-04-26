use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use super::buff;
use crate::state::battle::{
    fight_step::ActEffectBuilder,
    mechanics::empathy::{EMPATHY_BUFF_ID, EmpathyState},
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
    utils::effect_none,
};

const SOLACE_SKILL_IDS: [i32; 3] = [30800121, 30800122, 30800123];
const SOLACE_SELF_LOSS_PERMILLE: i32 = 100;
const SOLACE_CONFIG_EFFECT: i32 = 60039;

pub(super) struct Empathy;

impl BehaviorAction for Empathy {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        let BehaviorType::RealDamageSelfAndAddBuffToTarget { buff_id, .. } = behavior else {
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

        let self_damage = max_hp.saturating_mul(SOLACE_SELF_LOSS_PERMILLE) / 1000;
        let storage_amount = EmpathyState::compute_storage_amount(self_damage);
        let cap = EmpathyState::storage_cap(max_hp);
        let current_total = ctx.mechanics.empathy.apply_storage(
            &mut ctx.managers.buff_mgr,
            ctx.caster_uid,
            storage_amount,
            max_hp,
        );
        let buff_uid = ctx
            .managers
            .buff_mgr
            .find_instance_by_buff_id(ctx.caster_uid, EMPATHY_BUFF_ID)
            .map(|buff| buff.uid)
            .unwrap_or(0);

        let mut effects = vec![
            ctx.mechanics.empathy.emit_storage_injury(
                ctx.caster_uid,
                current_total,
                buff_uid,
                ctx.caster_uid,
                cap,
            ),
            ctx.mechanics.empathy.emit_buff_update(
                ctx.caster_uid,
                current_total,
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
