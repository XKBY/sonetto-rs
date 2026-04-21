use sonettobuf::{ActEffect, Fight};

use crate::state::battle::{context::FightContext, utils::buff_update};

pub(crate) fn is_preferred_defender_round_end_wrapper(fight: &Fight, effect: &ActEffect) -> bool {
    if effect.effect_type
        != Some(crate::state::battle::types::effects::EffectType::FightStep as i32)
    {
        return false;
    }
    let Some(fs) = effect.fight_step.as_ref() else {
        return false;
    };
    let from_uid = fs.from_id.unwrap_or(0);
    let to_uid = fs.to_id.unwrap_or(0);
    let act_id = fs.act_id.unwrap_or(0);
    let _ = fight;
    from_uid > 0 && to_uid < 0 && (4_000_000..5_000_000).contains(&act_id)
}

pub(crate) fn normalize_defender_round_end_wrapper(
    ctx: &FightContext<'_>,
    mut wrapper: ActEffect,
    broadcast_anchor_uid: Option<i64>,
) -> ActEffect {
    if !is_preferred_defender_round_end_wrapper(ctx.fight, &wrapper) {
        return wrapper;
    }
    let Some(fs) = wrapper.fight_step.as_mut() else {
        return wrapper;
    };
    let target_uid = fs.to_id.unwrap_or(0);
    let buff_id = fs.act_id.unwrap_or(0);
    if target_uid == 0 || buff_id == 0 {
        return wrapper;
    }

    if let Some(nested) = fs
        .act_effect
        .iter()
        .find(|e| {
            e.effect_type
                == Some(crate::state::battle::types::effects::EffectType::BuffUpdate as i32)
                && e.target_id == Some(target_uid)
                && e.buff.as_ref().and_then(|b| b.buff_id) == Some(buff_id)
        })
        .cloned()
    {
        fs.act_effect = vec![nested];
        return wrapper;
    }

    if let Some(instance) = ctx
        .managers
        .buff_mgr
        .get(target_uid)
        .iter()
        .filter(|b| b.buff_id == buff_id)
        .max_by_key(|b| b.uid)
    {
        let mut update = buff_update(
            target_uid,
            instance.from_uid,
            instance.buff_id,
            instance.uid,
            instance.stacks,
            instance.layer,
        );
        if let Some(info) = update.buff.as_mut() {
            info.duration = Some(0);
        }
        fs.act_effect = vec![update];
        return wrapper;
    }

    let mut synthesized_uid = broadcast_anchor_uid.map(|uid| uid.saturating_sub(4));
    if synthesized_uid == Some(0) {
        synthesized_uid = None;
    }
    let cfg = config::configs::get();
    let stacks = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| b.effect_count)
        .unwrap_or(0);
    let mut update = buff_update(
        target_uid,
        fs.from_id.unwrap_or(0),
        buff_id,
        synthesized_uid.unwrap_or(0),
        stacks,
        0,
    );
    if let Some(info) = update.buff.as_mut() {
        info.duration = Some(0);
    }
    fs.act_effect = vec![update];
    wrapper
}
