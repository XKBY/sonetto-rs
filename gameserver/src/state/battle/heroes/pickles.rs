//! Pickles — `The Dog Thinks` Insight passive plus the destiny
//! orphan passives (`30630151` / `30630161` / `30630171`) the data
//! tables don't list directly. The orphan ladder threads through
//! `crate::state::battle::destiny`; the inner-wrapper coalescer for
//! her Hedonism Implement (`30630122`) lives in `skill::executor`
//! today and will move here when the executor coalescer migrates.
//!
//! Round-end LIVE parity also needs a packet-shape repair: battle1
//! emits Pickles' Hedonism Implement as a dedicated top-level
//! `Effect -> 162 -> Effect -> 162 -> Skill` bundle before the final
//! round-sync marker, while our generic sweep currently leaks only a
//! late `30630171` wrapper into the flat broadcast step. The helper
//! below rewrites that tail into the exact Pickles-owned bundle and
//! corrects the live buff state to match the packet.

use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    event_queue::{HostSide, round_host_index_snapshot},
    fight_step::{FightStepBuilder, wrap_step},
    hero::HeroId,
    manager::buff_mgr::BuffMgr,
    round::step_shape::build_effect_step,
    step_walker,
};

const HEDONISM_IMPLEMENT_SKILL_ID: i32 = 30630151;
const HEDONISM_MARKER_SKILL_ID: i32 = 30630171;
const CLARIFIED_TOPIC_BUFF_ID: i32 = 30631;
const PICKLES_RECENT_ALLY_BUFF_IDS: [i32; 2] = [30630112, 30630113];
const HEDONISM_BUFF_IDS: [i32; 2] = [30630114, 30630115];

#[allow(dead_code)]
pub fn is_pickles(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Pickles.model_id())
}

pub fn repair_round_end_hedonism_emission(
    fight: &Fight,
    buff_mgr: &mut BuffMgr,
    steps: &mut Vec<FightStep>,
) -> bool {
    let Some(pickles) = fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .find(|entity| {
            is_pickles(entity.model_id)
                && entity.uid.unwrap_or(0) > 0
                && entity.position.unwrap_or(0) > 0
                && entity.current_hp.unwrap_or(0) > 0
                && entity.destiny_rank.unwrap_or(0) >= 1
        })
    else {
        return false;
    };
    let Some(pickles_uid) = pickles.uid else {
        return false;
    };

    if steps.iter().any(step_contains_pickles_hedonism_bundle) {
        return false;
    }

    let anchors = round_host_index_snapshot();
    let Some(target_uid) = fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .filter_map(|entity| {
            let uid = entity.uid?;
            if uid == pickles_uid
                || entity.position.unwrap_or(0) <= 0
                || entity.current_hp.unwrap_or(0) <= 0
            {
                return None;
            }

            let pickles_mark_count = PICKLES_RECENT_ALLY_BUFF_IDS
                .iter()
                .filter(|buff_id| {
                    buff_mgr
                        .find_instance_by_buff_id(uid, **buff_id)
                        .map(|instance| instance.from_uid == pickles_uid)
                        .unwrap_or(false)
                })
                .count();
            Some((pickles_mark_count, uid))
        })
        .filter(|(pickles_mark_count, _)| *pickles_mark_count > 0)
        .max_by_key(|(pickles_mark_count, uid)| (*pickles_mark_count, *uid))
        .map(|(_, uid)| uid)
        .or_else(|| {
            anchors
                .anchors_in_round()
                .iter()
                .filter(|anchor| {
                    anchor.side == HostSide::Player
                        && anchor.caster_uid > 0
                        && anchor.caster_uid != pickles_uid
                })
                .max_by_key(|anchor| (anchor.host_step_idx, anchor.act_order))
                .map(|anchor| anchor.caster_uid)
        })
    else {
        return false;
    };

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .iter()
            .any(|effect| effect.effect_type == Some(276))
    }) else {
        return false;
    };
    let Some(sync_idx) = steps
        .iter()
        .enumerate()
        .skip(round_end_idx + 1)
        .filter(|(_, step)| {
            step.act_type == Some(fight_step::ActType::Effect as i32)
                && step.act_effect.len() == 1
                && step.act_effect[0].effect_type == Some(310)
        })
        .map(|(idx, _)| idx)
        .last()
    else {
        return false;
    };

    let mut desired_self_update: Option<ActEffect> = None;
    let mut removed_marker_count: Option<i32> = None;
    let mut removed_any = false;
    let mut idx = round_end_idx + 1;
    while idx < sync_idx.min(steps.len()) {
        let step = &mut steps[idx];
        let mut effect_idx = 0;
        while effect_idx < step.act_effect.len() {
            if let Some(skill) =
                step_walker::wrapped_skill_from_effect(&step.act_effect[effect_idx])
                && skill.act_id == Some(HEDONISM_MARKER_SKILL_ID)
            {
                removed_marker_count = skill
                    .act_effect
                    .first()
                    .and_then(|effect| effect.buff.as_ref())
                    .and_then(|buff| buff.count);
                step.act_effect.remove(effect_idx);
                removed_any = true;
                continue;
            }

            let effect = &step.act_effect[effect_idx];
            if desired_self_update.is_none()
                && effect.effect_type == Some(7)
                && effect.target_id == Some(pickles_uid)
                && effect.buff.as_ref().and_then(|buff| buff.buff_id)
                    == Some(CLARIFIED_TOPIC_BUFF_ID)
            {
                desired_self_update = Some(effect.clone());
                step.act_effect.remove(effect_idx);
                continue;
            }
            effect_idx += 1;
        }

        if step.act_effect.is_empty() {
            steps.remove(idx);
            continue;
        }
        idx += 1;
    }

    if !removed_any {
        return false;
    }

    if let Some((buff_uid, current_stacks, current_layer)) = buff_mgr
        .find_instance_by_buff_id(pickles_uid, CLARIFIED_TOPIC_BUFF_ID)
        .map(|instance| (instance.uid, instance.stacks, instance.layer))
    {
        let fallback_count = removed_marker_count
            .unwrap_or(current_stacks)
            .saturating_sub(1);
        let desired_count = desired_self_update
            .as_ref()
            .and_then(|effect| effect.buff.as_ref())
            .and_then(|buff| buff.count)
            .unwrap_or(fallback_count);
        let desired_layer = desired_self_update
            .as_ref()
            .and_then(|effect| effect.buff.as_ref())
            .and_then(|buff| buff.layer)
            .unwrap_or(current_layer);
        let _ =
            buff_mgr.set_instance_count_layer(pickles_uid, buff_uid, desired_count, desired_layer);
        if desired_self_update.is_none() {
            desired_self_update = Some(
                crate::state::battle::fight_step::ActEffectBuilder::buff_update(
                    pickles_uid,
                    pickles_uid,
                    CLARIFIED_TOPIC_BUFF_ID,
                    buff_uid,
                    desired_count,
                    desired_layer,
                ),
            );
        }
    }

    let Some(self_update) = desired_self_update else {
        return false;
    };

    let mut hedonism_effects = Vec::with_capacity(4);
    for buff_id in HEDONISM_BUFF_IDS {
        let add = crate::state::battle::fight_step::ActEffectBuilder::buff_add_with_count(
            target_uid,
            pickles_uid,
            buff_id,
            0,
            0,
        );
        if let Some(buff) = add.buff.as_ref()
            && let Some(buff_uid) = buff.uid
        {
            buff_mgr.add_with_uid(target_uid, buff_id, pickles_uid, 0, 0, buff_uid);
        }
        hedonism_effects.push(add);
        hedonism_effects.push(ActEffect {
            effect_type: Some(26),
            target_id: Some(target_uid),
            effect_num: Some(0),
            ..Default::default()
        });
    }

    let hedonism_skill =
        FightStepBuilder::skill(pickles_uid, pickles_uid, HEDONISM_IMPLEMENT_SKILL_ID)
            .with_many(hedonism_effects)
            .build();
    let marker_skill = FightStepBuilder::skill(pickles_uid, pickles_uid, HEDONISM_MARKER_SKILL_ID)
        .with(self_update)
        .build();
    let bundle = build_effect_step(vec![wrap_step(build_effect_step(vec![
        wrap_step(hedonism_skill),
        wrap_step(marker_skill),
    ]))]);

    let insert_at = steps
        .iter()
        .enumerate()
        .skip(round_end_idx + 1)
        .filter(|(_, step)| {
            step.act_type == Some(fight_step::ActType::Effect as i32)
                && step.act_effect.len() == 1
                && step.act_effect[0].effect_type == Some(310)
        })
        .map(|(idx, _)| idx)
        .last()
        .unwrap_or(steps.len());
    steps.insert(insert_at, bundle);

    true
}

fn step_contains_pickles_hedonism_bundle(step: &FightStep) -> bool {
    step_contains_act_id(step, HEDONISM_IMPLEMENT_SKILL_ID)
        && step_contains_act_id(step, HEDONISM_MARKER_SKILL_ID)
}

fn step_contains_act_id(step: &FightStep, act_id: i32) -> bool {
    step.act_id == Some(act_id)
        || step.act_effect.iter().any(|effect| {
            effect
                .fight_step
                .as_ref()
                .map(|child| step_contains_act_id(child, act_id))
                .unwrap_or(false)
        })
}
