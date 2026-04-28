//! Enemy-actions phase: apply each enemy SkillEmit captured by the
//! AI deck to the round state and shape the resulting steps.
//!
//! For each AI step the phase:
//! 1. Sets up the per-action ex-gain prelude when the actor is a
//!    negative-uid SKILL caster.
//! 2. Applies the step to managers and snapshots the buff delta so
//!    deleted-buff ids feed `expand_trigger_chain`.
//! 3. Either lets `expand_trigger_chain` emit the result top-level
//!    (for non-host emissions) or routes the host through the
//!    magic-circle pre-embed + nested-skill embedding pipeline that
//!    matches the official client's nested-wrapper shape.
//!
//! Most of the per-action work calls back into `FightRoundMgr`'s
//! state-free helper methods. `mgr: &FightRoundMgr` is threaded in
//! purely for namespacing, since the unit struct holds no state.

use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{
    context::FightContext,
    manager::{card_mgr::FightCardMgr, round_mgr::FightRoundMgr},
    mechanics::magic_circle,
    passives::collector::CollectedPassives,
    round::RoundState,
    step_walker,
    steps::{ex_gain, trigger_embed},
    trigger::passes::sync_blood_value_baseline,
};

pub(crate) async fn run(
    mgr: &FightRoundMgr,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    card_mgr: &mut FightCardMgr,
    state: &mut RoundState,
    collected: &CollectedPassives,
    steps: &mut Vec<FightStep>,
) -> Result<()> {
    let battle_id = ctx.fight.battle_id.unwrap_or(0);
    sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
    sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
    state.enemy_skill_actors.clear();
    let ai_steps = card_mgr.execute_ai_turn(rng, ctx, state).await?;
    for step in ai_steps {
        let pre_skill_ex_step = if step.act_type == Some(fight_step::ActType::Skill as i32)
            && let Some(caster_uid) = step.from_id
            && caster_uid < 0
        {
            state.enemy_skill_actors.insert(caster_uid);
            ex_gain::standard_action_ex_gain_for_uid(mgr, ctx, caster_uid)
        } else {
            None
        };
        if let Some(ex_step) = pre_skill_ex_step {
            steps.push(ex_step);
        }

        let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
        mgr.apply_step_and_maybe_sync(ctx, &step, true)?;
        let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
        let runtime_deleted_buff_ids =
            mgr.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
        let is_embedded_skill_host = step.act_type == Some(fight_step::ActType::Skill as i32)
            && step.from_id.unwrap_or(0) >= 0;
        if !is_embedded_skill_host {
            let expanded_steps =
                mgr.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
            steps.extend(expanded_steps);
            continue;
        }

        let mut host_step = step.clone();
        step_walker::inline_magic_circle_root_wrapper(&mut host_step);
        magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
        let expanded_steps =
            mgr.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
        let preferred_nested_act_id = host_step.act_id.unwrap_or(0) - 20;
        let nested_skill_idx = host_step
            .act_effect
            .iter()
            .position(|e| {
                e.effect_type == Some(162)
                    && e.fight_step
                        .as_ref()
                        .map(|s| {
                            s.act_type == Some(fight_step::ActType::Skill as i32)
                                && s.act_id == Some(preferred_nested_act_id)
                        })
                        .unwrap_or(false)
            })
            .or_else(|| {
                host_step.act_effect.iter().rposition(|e| {
                    e.effect_type == Some(162)
                        && e.fight_step
                            .as_ref()
                            .map(|s| {
                                s.act_type == Some(fight_step::ActType::Skill as i32)
                                    && s.act_id != host_step.act_id
                            })
                            .unwrap_or(false)
                })
            });

        if let Some(idx) = nested_skill_idx {
            if let Some(nested) = host_step
                .act_effect
                .get_mut(idx)
                .and_then(|e| e.fight_step.as_mut())
            {
                // Inline pre-embeds (e.g. magic-circle aura follow-ups) can
                // surface a buff-granted passive as the chosen `nested`
                // wrapper. The combat-trigger pass then fires the same
                // passive again, and fallback-splices the duplicate into
                // `nested.act_effect`, producing a self-nested
                // act_id-in-act_id pair (e.g. 31260181 inside 31260181).
                // Drop any trigger whose SKILL id + from id match `nested`.
                let nested_act_id = nested.act_id;
                let nested_from_id = nested.from_id;
                let mut top_level_prefix: Vec<ActEffect> = Vec::new();
                let mut nested_embedded: Vec<ActEffect> = Vec::new();
                for trigger_step in expanded_steps.into_iter().skip(1) {
                    let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                    let duplicates_nested = embedded
                        .fight_step
                        .as_ref()
                        .map(|s| {
                            s.act_type == Some(fight_step::ActType::Skill as i32)
                                && s.act_id == nested_act_id
                                && s.from_id == nested_from_id
                        })
                        .unwrap_or(false);
                    if duplicates_nested {
                        continue;
                    }
                    let is_prep_prefix = embedded
                        .fight_step
                        .as_ref()
                        .map(|s| {
                            s.act_type == Some(fight_step::ActType::Skill as i32)
                                && s.from_id == host_step.from_id
                                && s.to_id == host_step.from_id
                                && s.act_id != host_step.act_id
                        })
                        .unwrap_or(false);
                    if is_prep_prefix {
                        top_level_prefix.push(embedded);
                    } else {
                        nested_embedded.push(embedded);
                    }
                }
                if !nested_embedded.is_empty() {
                    nested_embedded.sort_by_key(|e| {
                        let step = e.fight_step.as_ref();
                        let act_type = step.and_then(|s| s.act_type).unwrap_or(0);
                        if act_type == fight_step::ActType::Effect as i32 {
                            return 0;
                        }
                        let from = step.and_then(|s| s.from_id).unwrap_or(0);
                        if from < 0 { 1 } else { 2 }
                    });
                    let mut fallback_nested: Vec<ActEffect> = Vec::new();
                    for embedded in nested_embedded {
                        if !trigger_embed::insert_trigger_into_matching_nested(
                            nested,
                            embedded.clone(),
                        ) {
                            fallback_nested.push(embedded);
                        }
                    }
                    if !fallback_nested.is_empty() {
                        let insert_at = trigger_embed::find_trigger_insert_index(&nested.act_effect);
                        nested
                            .act_effect
                            .splice(insert_at..insert_at, fallback_nested);
                    }
                }
                if !top_level_prefix.is_empty() {
                    let mut merged = top_level_prefix;
                    merged.extend(std::mem::take(&mut host_step.act_effect));
                    host_step.act_effect = merged;
                }
            }
        } else {
            let mut embedded_steps: Vec<ActEffect> = Vec::new();
            for trigger_step in expanded_steps.into_iter().skip(1) {
                let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
                embedded_steps.push(embedded);
            }
            if !embedded_steps.is_empty() {
                let insert_at = step_walker::host_trigger_insert_index(&host_step);
                host_step
                    .act_effect
                    .splice(insert_at..insert_at, embedded_steps);
            }
        }
        trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
        trigger_embed::normalize_player_skill_effect_order(&mut host_step);
        steps.push(host_step);
    }
    Ok(())
}
