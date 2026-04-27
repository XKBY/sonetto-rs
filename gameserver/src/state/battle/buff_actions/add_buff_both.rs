use crate::state::battle::{
    fight_step::{effect_container_step, wrap_step},
    manager::buff_mgr::BuffInstance,
    skill::{
        SkillExecutor, buff,
        cache::resolve_skill_effect_id,
        targets::{alive_enemies_by_position, get_ally_uids},
    },
    types::condition::ConditionType,
};

use super::action::{BuffActCtx, BuffAction, BuffStage};
use super::result::ActionResult;

pub(super) struct AddBuffBothAction;

impl BuffAction for AddBuffBothAction {
    fn execute(
        &self,
        act_type: &str,
        parts: &[&str],
        ctx: &mut BuffActCtx<'_, '_>,
        stage: BuffStage,
    ) -> Option<ActionResult> {
        if stage == BuffStage::BeforeBuffAdd || act_type != "AddBuffBoth" {
            return None;
        }

        Some(apply(ctx, parts))
    }
}

fn apply(ctx: &mut BuffActCtx<'_, '_>, parts: &[&str]) -> ActionResult {
    let buff_a = parts
        .get(1)
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let buff_b = parts
        .get(3)
        .and_then(|v| v.trim().parse::<i32>().ok())
        .unwrap_or(0);
    if buff_a <= 0 && buff_b <= 0 {
        return ActionResult::empty();
    }

    let original_target = ctx.effect_ctx.target;
    let caster_uid = ctx.effect_ctx.caster_uid();
    let skill_id = ctx
        .executor
        .current_skill_context()
        .map(|(skill_id, _)| skill_id)
        .unwrap_or(0);
    let has_bloodpool = ctx.has_bloodpool;
    let fight = ctx.effect_ctx.fight;
    let mut inner_effects = Vec::new();

    for target_uid in
        add_buff_both_targets(ctx.executor, fight, caster_uid, original_target, buff_a)
    {
        let managers = &mut *ctx.effect_ctx.managers;
        let mechanics = &mut *ctx.effect_ctx.mechanics;
        let mut effects = buff::apply(
            ctx.executor,
            fight,
            managers,
            mechanics,
            caster_uid,
            target_uid,
            buff_a,
            0,
            has_bloodpool,
            skill_id,
            0,
            &ConditionType::None,
        );
        hydrate_buff_effects(
            managers.buff_mgr.get(target_uid),
            buff_a,
            caster_uid,
            &mut effects,
        );
        inner_effects.extend(effects);
    }

    for target_uid in
        add_buff_both_targets(ctx.executor, fight, caster_uid, original_target, buff_b)
    {
        let managers = &mut *ctx.effect_ctx.managers;
        let mechanics = &mut *ctx.effect_ctx.mechanics;
        let mut effects = buff::apply(
            ctx.executor,
            fight,
            managers,
            mechanics,
            caster_uid,
            target_uid,
            buff_b,
            0,
            has_bloodpool,
            skill_id,
            0,
            &ConditionType::None,
        );
        hydrate_buff_effects(
            managers.buff_mgr.get(target_uid),
            buff_b,
            caster_uid,
            &mut effects,
        );
        inner_effects.extend(effects);
    }

    if inner_effects.is_empty() {
        return ActionResult::empty();
    }

    ActionResult::single(wrap_step(effect_container_step(
        caster_uid,
        original_target,
        ctx.buff_id,
        inner_effects,
    )))
}

fn add_buff_both_targets(
    executor: &SkillExecutor,
    fight: &sonettobuf::Fight,
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
    }

    if is_good_buff(child_buff_id) {
        let allies = get_ally_uids(fight, caster_uid);
        if !allies.is_empty() {
            return allies;
        }
    }

    vec![original_target]
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
