//! Sentinel Hour-of-Repentance: a channeled reactive that answers
//! battle-rule-derived boss skills with a counter-attack
//! (skill 31260171).
//!
//! While the channel-state buff (31260131) is active and the layer
//! counter buff (31260151) has at least one stack, Sentinel will
//! react to enemy SKILL wrappers whose act_id is in
//! `BOSS_TRIGGER_SKILLS`. The reactive's effect_container_step is
//! spliced into the host wrapper's act_effect immediately before its
//! BuffUpdate marker, so the host remains well-formed. After the
//! reactive resolves, one layer of the channel buff is consumed.
//!
//! In-game text describes this as Sentinel 'interrupting' or
//! 'responding to' the enemy's casting; the engine implements that
//! as a 162-wrapped child grafted onto the boss's outgoing emission.

use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    fight_step::{effect_container_step, wrap_step},
    manager::buff_mgr::BuffInstance,
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    skill::PhaseFilter,
    steps::trigger_embed,
    types::effects::EffectType,
};

/// Channel-state buff that gates the reactive. Also reused as the
/// act_id of the response's effect_container wrapper.
pub const CHANNEL_STATE_BUFF_ID: i32 = 31260131;

/// Layer-counter buff that tracks remaining reactive uses.
pub const CHANNEL_LAYER_BUFF_ID: i32 = 31260151;

/// Sentinel's reactive counter-attack skill.
pub const REACTIVE_SKILL_ID: i32 = 31260171;

/// Battle-rule-derived enemy skills that the reactive answers. These
/// are sourced from `rule.json::effect`; see Phase 3's
/// `is_battle_rule_derived` cache. They're listed verbatim here for
/// audit clarity, not because the engine should hardcode the family
/// - a future migration will resolve them via rule lookup.
pub const BOSS_TRIGGER_SKILLS: [i32; 2] = [530000721, 530000752];

/// Find the alive ally currently channelling Hour of Repentance, if
/// any. Returns the ally uid plus a snapshot of the layer-counter
/// buff so callers can decrement it after the reactive resolves.
pub fn active_holder(ctx: &FightContext<'_>) -> Option<(i64, BuffInstance)> {
    let attacker = ctx.fight.attacker.as_ref()?;
    attacker
        .entitys
        .iter()
        .chain(attacker.sub_entitys.iter())
        .filter(|entity| entity.current_hp.unwrap_or(0) > 0)
        .filter_map(|entity| entity.uid)
        .find_map(|uid| {
            let has_channel_state = ctx.managers.buff_mgr.has(uid, CHANNEL_STATE_BUFF_ID);
            let channel_buff = ctx
                .managers
                .buff_mgr
                .get(uid)
                .iter()
                .find(|buff| {
                    buff.buff_id == CHANNEL_LAYER_BUFF_ID && buff.layer.max(buff.stacks).max(0) >= 1
                })
                .cloned();
            if has_channel_state {
                channel_buff.map(|buff| (uid, buff))
            } else {
                None
            }
        })
}

/// Pick the splice index inside a host SKILL wrapper's act_effect
/// that keeps the host well-formed: insert right before the trailing
/// BuffUpdate marker, or at the end if no marker is present.
pub fn splice_index(effects: &[ActEffect]) -> usize {
    effects
        .iter()
        .rposition(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
        .unwrap_or(effects.len())
}

/// Decrement one layer of the channel buff after a reactive
/// resolution. Mirrors the original BuffMgr write order so the
/// runtime layer/stack accounting stays identical.
pub fn consume_layer(ctx: &mut FightContext<'_>, holder_uid: i64, buff: &BuffInstance) {
    let new_layer = if buff.layer > 0 {
        buff.layer.saturating_sub(1)
    } else {
        0
    };
    let new_stacks = if buff.layer > 0 {
        buff.stacks
    } else {
        buff.stacks.saturating_sub(1)
    };
    ctx.managers.buff_mgr.add_with_uid(
        holder_uid,
        buff.buff_id,
        buff.from_uid,
        new_stacks,
        new_layer,
        buff.uid,
    );
}

/// Walk a freshly built boss subtree and graft Sentinel's reactive
/// answer onto every eligible SKILL wrapper. Called after the round
/// manager materializes the subtree but before it serializes upward.
///
/// The recursive descent matches the original (`for effect in
/// boss_subtree.iter_mut()` with a self-recursive call on
/// `step.act_effect`). The `expand_trigger_chain` call on the
/// reactive's container step continues to flow through the Phase
/// 5a-narrow EventQueue plumbing - `RoundManager::expand_trigger_chain`
/// is reused via the function reference passed in.
pub fn graft_reactives_onto_boss_subtree<F, G>(
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
    boss_subtree: &mut Vec<ActEffect>,
    expand_trigger_chain: &F,
    deleted_buff_ids_from_delta: &G,
) where
    F: Fn(&mut FightContext<'_>, &CollectedPassives, &FightStep, &[i32]) -> Vec<FightStep>,
    G: Fn(&[(i64, BuffInstance)], &[(i64, BuffInstance)]) -> Vec<i32>,
{
    for effect in boss_subtree.iter_mut() {
        let Some(step) = effect.fight_step.as_mut() else {
            continue;
        };
        graft_reactives_onto_boss_subtree(
            ctx,
            collected,
            &mut step.act_effect,
            expand_trigger_chain,
            deleted_buff_ids_from_delta,
        );

        if effect.effect_type != Some(162)
            || step.act_type != Some(fight_step::ActType::Skill as i32)
            || step.from_id.unwrap_or(0) >= 0
            || !BOSS_TRIGGER_SKILLS.contains(&step.act_id.unwrap_or(0))
        {
            continue;
        }

        let Some((holder_uid, channel_buff)) = active_holder(ctx) else {
            continue;
        };
        let enemy_caster_uid = step.from_id.unwrap_or(0);
        let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
        let Ok(skill_effects) = execute_passive_skill(
            ctx,
            holder_uid,
            enemy_caster_uid,
            REACTIVE_SKILL_ID,
            &PhaseFilter::combat(),
        ) else {
            continue;
        };
        if skill_effects.is_empty() {
            continue;
        }
        let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
        let runtime_deleted_buff_ids =
            deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

        let mut sentinel_step = effect_container_step(
            holder_uid,
            enemy_caster_uid,
            CHANNEL_STATE_BUFF_ID,
            skill_effects,
        );
        let expanded_steps =
            expand_trigger_chain(ctx, collected, &sentinel_step, &runtime_deleted_buff_ids);
        let mut fallback_nested: Vec<ActEffect> = Vec::new();
        for trigger_step in expanded_steps.into_iter().skip(1) {
            let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
            if !trigger_embed::insert_trigger_into_matching_nested(
                &mut sentinel_step,
                embedded.clone(),
            ) {
                fallback_nested.push(embedded);
            }
        }
        if !fallback_nested.is_empty() {
            let insert_at = trigger_embed::find_trigger_insert_index(&sentinel_step.act_effect);
            sentinel_step
                .act_effect
                .splice(insert_at..insert_at, fallback_nested);
        }

        let sentinel_wrapper = wrap_step(sentinel_step);
        let insert_at = splice_index(&step.act_effect);
        step.act_effect.insert(insert_at, sentinel_wrapper);
        consume_layer(ctx, holder_uid, &channel_buff);
    }
}
