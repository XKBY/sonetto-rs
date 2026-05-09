use crate::state::battle::fight_step::{ActEffectBuilder, effect_container_step, wrap_step};
use crate::state::battle::manager::buff_mgr::BuffInstance;
use crate::state::battle::skill::get_entity;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::apply_real_hurt_fix;

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

const CRIT_PERMILLE: i32 = 1390;

#[derive(Clone)]
pub(super) struct DotTickParams {
    owner_uid: i64,
    carrier: Option<BuffInstance>,
    marker_effect_type: i32,
    damage: i32,
}

#[derive(Clone)]
pub(super) struct BurnTickParams {
    owner_uid: i64,
    carrier: Option<BuffInstance>,
    damage: i32,
}

pub(super) struct PoisonHandler;
pub(super) struct DeadlyPoisonHandler;
pub(super) struct BurnHandler;

impl BuffActionHandler for PoisonHandler {
    type Params = DotTickParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Poison" && stage == BuffStage::RoundEndDot
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        dot_tick_params(parts, ctx, EffectType::Poison as i32)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        poison_family_result(params)
    }
}

impl BuffActionHandler for DeadlyPoisonHandler {
    type Params = DotTickParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "DeadlyPoison" && stage == BuffStage::RoundEndDot
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        dot_tick_params(parts, ctx, EffectType::DeadlyPoison as i32)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        poison_family_result(params)
    }
}

impl BuffActionHandler for BurnHandler {
    type Params = BurnTickParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "Burn" && stage == BuffStage::RoundEndDot
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        burn_tick_params(parts, ctx)
    }

    fn steps(&self, params: Self::Params, _ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        burn_result(params)
    }
}

fn dot_tick_params(
    parts: &[&str],
    ctx: &BuffActCtx<'_, '_>,
    marker_effect_type: i32,
) -> DotTickParams {
    let permille = parse_i32(parts, 1);
    let damage = ctx
        .carrier
        .as_ref()
        .map(|carrier| compute_origin_crit_damage(ctx, carrier, permille))
        .unwrap_or(0);

    DotTickParams {
        owner_uid: ctx.owner_uid,
        carrier: ctx.carrier.clone(),
        marker_effect_type,
        damage,
    }
}

fn poison_family_result(params: DotTickParams) -> ActionResult {
    let Some(carrier) = params.carrier else {
        return ActionResult::empty();
    };
    if params.owner_uid == 0 || params.damage <= 0 {
        return ActionResult::empty();
    }

    let mut effects = Vec::new();
    let stacks = carrier.layer.max(1);
    for _ in 0..stacks {
        let inner = effect_container_step(
            carrier.from_uid,
            params.owner_uid,
            dot_wrapper_act_id(carrier.buff_id),
            vec![
                ActEffectBuilder::new(params.marker_effect_type, params.owner_uid)
                    .effect_num(carrier.buff_id)
                    .build(),
                ActEffectBuilder::new(EffectType::OriginCrit as i32, params.owner_uid)
                    .effect_num(params.damage)
                    .build(),
            ],
        );
        effects.push(wrap_step(inner));
    }

    ActionResult::effects(effects)
}

fn burn_tick_params(parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> BurnTickParams {
    let rate = parse_i32(parts, 1);
    let attr_id = parse_i32(parts, 2);
    let damage = ctx
        .carrier
        .as_ref()
        .map(|carrier| compute_burn_damage(ctx, carrier, rate, attr_id))
        .unwrap_or(0);

    BurnTickParams {
        owner_uid: ctx.owner_uid,
        carrier: ctx.carrier.clone(),
        damage,
    }
}

fn burn_result(params: BurnTickParams) -> ActionResult {
    let Some(carrier) = params.carrier else {
        return ActionResult::empty();
    };
    if params.owner_uid == 0 || params.damage <= 0 {
        return ActionResult::empty();
    }

    let mut effects = Vec::new();
    let stacks = carrier.layer.max(1);
    for _ in 0..stacks {
        let inner = effect_container_step(
            carrier.from_uid,
            params.owner_uid,
            carrier.buff_id,
            vec![
                ActEffectBuilder::marker(
                    EffectType::Burn as i32,
                    params.owner_uid,
                    carrier.buff_id,
                ),
                ActEffectBuilder::origin_damage(params.owner_uid, params.damage, None),
            ],
        );
        effects.push(wrap_step(inner));
    }

    ActionResult::effects(effects)
}

fn compute_origin_crit_damage(
    ctx: &BuffActCtx<'_, '_>,
    carrier: &BuffInstance,
    permille: i32,
) -> i32 {
    let Some(caster) = get_entity(ctx.effect_ctx.fight(), carrier.from_uid) else {
        return 0;
    };
    let caster_atk = caster
        .attr
        .as_ref()
        .and_then(|attr| attr.attack)
        .unwrap_or(0);
    if caster_atk <= 0 || permille <= 0 {
        return 0;
    }
    let base_damage = apply_real_hurt_fix(
        &ctx.effect_ctx.managers.buff_mgr,
        ctx.owner_uid,
        caster_atk * permille / 1000,
    );
    if base_damage <= 0 {
        return 0;
    }
    base_damage.saturating_mul(CRIT_PERMILLE) / 1000
}

fn compute_burn_damage(
    ctx: &BuffActCtx<'_, '_>,
    carrier: &BuffInstance,
    rate: i32,
    attr_id: i32,
) -> i32 {
    let Some(caster) = get_entity(ctx.effect_ctx.fight(), carrier.from_uid) else {
        return 0;
    };
    let source_value = lookup_attr(caster, attr_id);
    if source_value <= 0 || rate <= 0 {
        return 0;
    }
    apply_real_hurt_fix(
        &ctx.effect_ctx.managers.buff_mgr,
        ctx.owner_uid,
        source_value * rate / 1000,
    )
}

fn lookup_attr(entity: &sonettobuf::FightEntityInfo, attr_id: i32) -> i32 {
    let attr = entity.attr.as_ref();
    match attr_id {
        100 => entity.current_hp.unwrap_or(0),
        101 => attr.and_then(|a| a.hp).unwrap_or(0),
        102 => attr.and_then(|a| a.attack).unwrap_or(0),
        103 => attr.and_then(|a| a.defense).unwrap_or(0),
        _ => attr.and_then(|a| a.attack).unwrap_or(0),
    }
}

fn dot_wrapper_act_id(buff_id: i32) -> i32 {
    match buff_id {
        30980132 => 0,
        _ => buff_id,
    }
}

fn parse_i32(parts: &[&str], idx: usize) -> i32 {
    parts
        .get(idx)
        .and_then(|part| part.trim().parse::<i32>().ok())
        .unwrap_or(0)
}
