use crate::state::battle::{
    fight_step::{effect_container_step, wrap_step},
    manager::buff_mgr::BuffInstance,
    skill::{
        SkillExecutor, buff,
        cache::resolve_skill_effect_id,
        targets::{alive_enemies_by_position, get_ally_uids},
    },
};

use super::action::{BuffActCtx, BuffActionHandler, BuffStage};
use super::result::ActionResult;

pub(super) struct AddBuffBothParams {
    pub buff_a: i32,
    pub buff_b: i32,
    pub original_target: i64,
    pub caster_uid: i64,
    pub skill_id: i32,
    pub has_bloodpool: bool,
    pub inner_effects: Vec<sonettobuf::ActEffect>,
    pub emit_empty_wrapper_on_noop: bool,
}

pub(super) struct AddBuffBothHandler;

impl BuffActionHandler for AddBuffBothHandler {
    type Params = AddBuffBothParams;

    fn matches(&self, act_type: &str, stage: BuffStage) -> bool {
        act_type == "AddBuffBoth" && stage == BuffStage::AfterBuffAdd
    }

    fn parse(&self, parts: &[&str], ctx: &BuffActCtx<'_, '_>) -> Self::Params {
        let buff_a = parts
            .get(1)
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        let buff_b = parts
            .get(3)
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        AddBuffBothParams {
            buff_a,
            buff_b,
            original_target: ctx.effect_ctx.target,
            caster_uid: ctx.effect_ctx.caster_uid(),
            skill_id: ctx
                .executor
                .current_skill_context()
                .map(|(skill_id, _)| skill_id)
                .unwrap_or(0),
            has_bloodpool: ctx.has_bloodpool,
            inner_effects: Vec::new(),
            emit_empty_wrapper_on_noop: false,
        }
    }

    fn execute(&self, params: &mut Self::Params, ctx: &mut BuffActCtx<'_, '_>) {
        if params.buff_a <= 0 && params.buff_b <= 0 {
            return;
        }
        if should_skip_add_buff_both_update(params, ctx) {
            params.emit_empty_wrapper_on_noop = true;
            return;
        }

        let fight = ctx.effect_ctx.fight;
        for target_uid in add_buff_both_targets(
            ctx.executor,
            fight,
            &ctx.effect_ctx.managers.buff_mgr,
            params.caster_uid,
            params.original_target,
            params.buff_a,
        ) {
            apply_child_buff(params, ctx, target_uid, params.buff_a);
        }

        for target_uid in add_buff_both_targets(
            ctx.executor,
            fight,
            &ctx.effect_ctx.managers.buff_mgr,
            params.caster_uid,
            params.original_target,
            params.buff_b,
        ) {
            apply_child_buff(params, ctx, target_uid, params.buff_b);
        }
    }

    fn steps(&self, params: Self::Params, ctx: &BuffActCtx<'_, '_>) -> ActionResult {
        if params.inner_effects.is_empty() {
            if params.emit_empty_wrapper_on_noop {
                return ActionResult::single(wrap_step(effect_container_step(
                    params.caster_uid,
                    params.original_target,
                    ctx.buff_id,
                    Vec::new(),
                )));
            }
            return ActionResult::empty();
        }
        ActionResult::single(wrap_step(effect_container_step(
            params.caster_uid,
            params.original_target,
            ctx.buff_id,
            params.inner_effects,
        )))
    }
}

fn should_skip_add_buff_both_update(
    params: &AddBuffBothParams,
    ctx: &BuffActCtx<'_, '_>,
) -> bool {
    if params.buff_b <= 0 {
        return false;
    }

    let holder_layer = ctx
        .effect_ctx
        .managers
        .buff_mgr
        .get(params.original_target)
        .iter()
        .filter(|instance| instance.buff_id == ctx.buff_id && instance.from_uid == params.caster_uid)
        .max_by_key(|instance| instance.uid)
        .map(|instance| instance.layer)
        .unwrap_or(0);
    if holder_layer != 3 {
        return false;
    }

    let buff_b_targets = add_buff_both_targets(
        ctx.executor,
        ctx.effect_ctx.fight,
        &ctx.effect_ctx.managers.buff_mgr,
        params.caster_uid,
        params.original_target,
        params.buff_b,
    );
    !buff_b_targets.is_empty()
        && buff_b_targets.iter().all(|target_uid| {
            ctx.effect_ctx
                .managers
                .buff_mgr
                .get(*target_uid)
                .iter()
                .any(|instance| instance.buff_id == params.buff_b && instance.from_uid == params.caster_uid)
        })
}

fn apply_child_buff(
    params: &mut AddBuffBothParams,
    ctx: &mut BuffActCtx<'_, '_>,
    target_uid: i64,
    buff_id: i32,
) {
    if buff_id <= 0 {
        return;
    }
    let fight = ctx.effect_ctx.fight;
    let managers = &mut *ctx.effect_ctx.managers;
    let mechanics = &mut *ctx.effect_ctx.mechanics;
    let mut effects = buff::apply(
        buff::BuffApplySpec::new(buff_id)
            .caster(params.caster_uid)
            .target(target_uid)
            .bloodpool(params.has_bloodpool)
            .skill(params.skill_id),
        ctx.executor,
        fight,
        managers,
        mechanics,
    );
    hydrate_buff_effects(
        managers.buff_mgr.get(target_uid),
        buff_id,
        params.caster_uid,
        &mut effects,
    );
    params.inner_effects.extend(effects);
}

fn add_buff_both_targets(
    executor: &SkillExecutor,
    fight: &sonettobuf::Fight,
    buff_store: &crate::state::battle::manager::buff_mgr::BuffMgr,
    caster_uid: i64,
    original_target: i64,
    child_buff_id: i32,
) -> Vec<i64> {
    let self_targeted = original_target == caster_uid;
    if !self_targeted {
        return vec![original_target];
    }

    if is_bad_buff(child_buff_id) {
        let hostile_targets = current_hostile_targets(executor, fight, caster_uid);
        if !hostile_targets.is_empty() {
            return hostile_targets;
        }
        let poisoned_targets = poisoned_enemy_targets(buff_store, fight, caster_uid);
        if !poisoned_targets.is_empty() {
            return poisoned_targets;
        }
        // No hostile targets for a bad buff — drop, don't fall back to self.
        // Otherwise AddBuffBoth applies the debuff to the caster (e.g.
        // Sotheby poisoning herself when buff 30091120's AddBuffBoth fires
        // outside a hostile-skill context), and each fire stacks a fresh
        // self-poison instance on the caster.
        return Vec::new();
    }

    if is_good_buff(child_buff_id) {
        let allies = get_ally_uids(fight, caster_uid);
        if !allies.is_empty() {
            return allies;
        }
    }

    vec![original_target]
}

fn poisoned_enemy_targets(
    buff_store: &crate::state::battle::manager::buff_mgr::BuffMgr,
    fight: &sonettobuf::Fight,
    caster_uid: i64,
) -> Vec<i64> {
    alive_enemies_by_position(fight, caster_uid)
        .into_iter()
        .filter(|target_uid| {
            buff_store.get(*target_uid).iter().any(|instance| {
                instance.from_uid == caster_uid && is_poison_family(instance.buff_id)
            })
        })
        .collect()
}

fn is_poison_family(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|row| row.id == buff_id)
        .map(|row| {
            row.features.split('|').any(|entry| {
                entry
                    .split('#')
                    .next()
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    .is_some_and(|act_id| matches!(act_id, 803 | 844))
            })
        })
        .unwrap_or(false)
}

fn current_hostile_targets(
    executor: &SkillExecutor,
    fight: &sonettobuf::Fight,
    caster_uid: i64,
) -> Vec<i64> {
    let Some((skill_id, selected_target_uid)) = executor.current_skill_context() else {
        return Vec::new();
    };
    if selected_target_uid == 0 || selected_target_uid.signum() == caster_uid.signum() {
        return Vec::new();
    }

    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let logic_target = cfg
        .skill_effect
        .iter()
        .find(|row| row.id == effect_id)
        .and_then(|row| row.logic_target.trim().parse::<i32>().ok())
        .unwrap_or(0);

    let mut targets = match logic_target {
        201 => {
            let enemies = alive_enemies_by_position(fight, caster_uid);
            if enemies.is_empty() {
                vec![selected_target_uid]
            } else if let Some(idx) = enemies.iter().position(|uid| *uid == selected_target_uid) {
                let mut out = vec![selected_target_uid];
                if enemies.len() > 1 {
                    let extra = enemies[(idx + 1) % enemies.len()];
                    if extra != selected_target_uid {
                        out.push(extra);
                    }
                }
                out
            } else {
                enemies.into_iter().take(2).collect()
            }
        }
        202 | 301 | 302 => alive_enemies_by_position(fight, caster_uid),
        _ => vec![selected_target_uid],
    };
    targets.retain(|uid| *uid != 0 && uid.signum() != caster_uid.signum());
    targets.dedup();
    targets
}

fn is_good_buff(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|row| row.id == buff_id)
        .map(|row| row.is_good_buff == 1)
        .unwrap_or(false)
}

fn is_bad_buff(buff_id: i32) -> bool {
    config::configs::get()
        .skill_buff
        .iter()
        .find(|row| row.id == buff_id)
        .map(|row| row.is_good_buff == 2)
        .unwrap_or(false)
}

fn hydrate_buff_effects(
    runtime_buffs: &[BuffInstance],
    buff_id: i32,
    from_uid: i64,
    effects: &mut [sonettobuf::ActEffect],
) {
    let runtime = runtime_buffs
        .iter()
        .filter(|instance| instance.buff_id == buff_id && instance.from_uid == from_uid)
        .max_by_key(|instance| instance.uid);
    let Some(runtime) = runtime else {
        return;
    };

    for effect in effects {
        let effect_type = effect.effect_type.unwrap_or(0);
        if !matches!(
            effect_type,
            x if x == crate::state::battle::types::effects::EffectType::BuffAdd as i32
                || x == crate::state::battle::types::effects::EffectType::BuffUpdate as i32
        ) {
            continue;
        }
        let Some(buff) = effect.buff.as_mut() else {
            continue;
        };
        if buff.buff_id != Some(buff_id) || buff.from_uid != Some(from_uid) {
            continue;
        }
        if buff.uid.unwrap_or(0) == 0 {
            buff.uid = Some(runtime.uid);
        }
        if buff.duration.unwrap_or(0) == 0 && runtime.duration > 0 {
            buff.duration = Some(runtime.duration);
        }
    }
}
