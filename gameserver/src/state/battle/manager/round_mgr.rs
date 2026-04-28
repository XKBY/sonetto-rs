use anyhow::Result;
use once_cell::sync::Lazy;
use rand::rngs::StdRng;
use sonettobuf::{ActEffect, BeginRoundOper, CardInfo, Fight, FightRound, FightStep, fight_step};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use super::super::{
    ConditionType,
    buff_actions::blood_pool_ex::build_blood_pool_gain_ex_point_step,
    buff_actions::round_end as round_end_handler,
    card::CardOpType,
    context::{FightContext, RoundContext},
    event_queue::{EventContext, EventQueue, drain_to_fight_steps, fight_step_to_event},
    fight_step::{
        ActEffectBuilder, FightStepBuilder, effect_container_step, make_skill_step,
        split_step_by_effect_limit, wrap_step,
    },
    manager::{
        buff_mgr::{
            BuffMgr, DEFENDER_BUFF_UID_START, attacker_buff_uid_checkpoint,
            defender_buff_uid_checkpoint, next_buff_uid_for_target, reset_buff_uid_to,
            sync_buff_uid_counters_from_mgr,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        card_mgr::FightCardMgr,
        ex_point_mgr::{ExPointMgr, build_ex_point_info, sync_from_fight, sync_to_fight},
        traits::Manager,
        wave_spawn,
    },
    mechanics::{
        advanced_cure, bloodtithe, channel as channel_mechanics, dot, injury_counter, magic_circle,
        nautika_psychube_bundle,
    },
    passives::{
        collector::{CollectedPassives, collect},
        steps::skill::execute_skill as execute_passive_skill,
    },
    round::{
        PassivePhaseConfig, PhaseDepth, PhaseScope, PhaseSkillSet, PhaseStepShape, RoundState,
        step_shape::{build_effect_step, split_updates_and_wrap_rest},
        steps::{refresh::build_refresh_step, transitions::build_pre_enemy_transition_steps},
    },
    skill::{
        PhaseFilter, SkillExecutor,
        cache::resolve_skill_effect_id,
        classification::{CombatPassiveScanMode, has_combat_reactive_condition},
        condition::{misc::HriEvalGuard, parser::parse_condition},
        euphoria::resolve_with_euphoria,
    },
    step_walker,
    steps::{broadcast, ex_gain, step_normalize, trigger_embed},
    trigger::{
        combat::{event_from_step, fire_combat_triggers},
        passes::{build_belief_gain_step, sync_blood_value_baseline},
    },
    types::effects::EffectType,
    utils::buff_del,
};

enum BattleEndState {
    Ongoing,
    WaveCleared, // all enemies dead, more waves remain
    Victory,     // all waves cleared
    Defeat,      // all heroes dead
}

struct RoundOpenPhaseData {
    state: RoundState,
    steps: Vec<FightStep>,
    collected: CollectedPassives,
    selected_for_round_end: Vec<CardInfo>,
    selected_non_temp: Vec<CardInfo>,
    deck_num: i32,
    defender_uid_checkpoint: i64,
}

static ENTRY_MAX_HP: Lazy<Mutex<HashMap<(i32, i64), i32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn condition_has_matching_hero_round_interval(condition: &ConditionType, cur_round: i32) -> bool {
    match condition {
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => conds
            .iter()
            .any(|cond| condition_has_matching_hero_round_interval(cond, cur_round)),
        ConditionType::HeroRoundInterval {
            start_round,
            period,
        } => crate::state::battle::skill::condition::misc::hero_round_interval_matches(
            *start_round,
            *period,
            cur_round,
        ),
        _ => false,
    }
}

fn skill_carries_hero_round_interval(skill_id: i32, cur_round: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill_cfg) = config::configs::get().skill_effect.get(effect_id) else {
        return false;
    };

    [
        skill_cfg.condition1.as_str(),
        skill_cfg.condition2.as_str(),
        skill_cfg.condition3.as_str(),
        skill_cfg.condition4.as_str(),
        skill_cfg.condition5.as_str(),
        skill_cfg.condition6.as_str(),
    ]
    .into_iter()
    .filter(|raw| !raw.trim().is_empty())
    .any(|raw| {
        let (condition, _) = parse_condition(raw.trim());
        condition_has_matching_hero_round_interval(&condition, cur_round)
    })
}

fn skill_has_no_act_round_condition(skill_id: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill_cfg) = config::configs::get().skill_effect.get(effect_id) else {
        return false;
    };

    [
        skill_cfg.condition1.as_str(),
        skill_cfg.condition2.as_str(),
        skill_cfg.condition3.as_str(),
        skill_cfg.condition4.as_str(),
        skill_cfg.condition5.as_str(),
        skill_cfg.condition6.as_str(),
    ]
    .into_iter()
    .filter(|raw| !raw.trim().is_empty())
    .any(|raw| {
        let (condition, _) = parse_condition(raw.trim());
        crate::state::battle::skill::condition::fold(&condition, &mut |cond| {
            matches!(cond, ConditionType::NoActRound)
        })
    })
}

/// Whether `effect` wraps a FightStep whose act_effect is entirely
/// display-only markers (BuffUpdate/Attr with effect_num=0). Defender idle
/// sweeps use this to drop state-machine re-ticks that LIVE only emits
/// nested inside real combat events.
fn is_marker_only_fight_step_effect(effect: &ActEffect) -> bool {
    let Some(step) = effect.fight_step.as_ref() else {
        return false;
    };
    if step.act_effect.is_empty() {
        return false;
    }
    step.act_effect.iter().all(|inner| {
        let et = inner.effect_type.unwrap_or(0);
        let num = inner.effect_num.unwrap_or(0);
        (et == EffectType::BuffUpdate as i32 && num == 0)
            || (et == EffectType::Attr as i32 && num == 0)
    })
}

pub(crate) fn seed_entry_max_hp_from_fight(fight: &Fight) {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 {
        return;
    }
    let mut tracker = ENTRY_MAX_HP.lock().expect("entry max hp mutex poisoned");
    for entity in fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|side| side.entitys.iter().chain(side.sub_entitys.iter())),
        )
    {
        let Some(uid) = entity.uid else {
            continue;
        };
        tracker.entry((battle_id, uid)).or_insert_with(|| {
            entity
                .base_attr
                .as_ref()
                .and_then(|a| a.hp)
                .or_else(|| entity.attr.as_ref().and_then(|a| a.hp))
                .unwrap_or(entity.current_hp.unwrap_or(0))
                .max(entity.current_hp.unwrap_or(0))
        });
    }
}

pub(crate) fn lookup_entry_max_hp(fight: &Fight, uid: i64) -> i32 {
    let battle_id = fight.battle_id.unwrap_or(0);
    if battle_id == 0 || uid == 0 {
        return 0;
    }
    ENTRY_MAX_HP
        .lock()
        .expect("entry max hp mutex poisoned")
        .get(&(battle_id, uid))
        .copied()
        .unwrap_or(0)
}

fn active_cloth_level(fight: &Fight) -> Option<config::cloth_level::ClothLevel> {
    let cloth_id = fight
        .attacker
        .as_ref()
        .and_then(|attacker| attacker.cloth_id)?;
    config::configs::get()
        .cloth_level
        .iter()
        .find(|cloth| cloth.id == cloth_id && cloth.level == 1)
        .cloned()
}

fn parse_cloth_recover_delta(recover: &str, round_index: i32) -> i32 {
    recover
        .split('|')
        .filter_map(|entry| {
            let mut parts = entry.trim().split('#');
            let start_round = parts.next()?.trim().parse::<i32>().ok()?;
            let amount = parts.next()?.trim().parse::<i32>().ok()?;
            if parts.next().is_some() {
                return None;
            }
            Some((start_round, amount))
        })
        .filter(|(start_round, _)| *start_round == round_index)
        .map(|(_, amount)| amount.max(0))
        .sum()
}

fn seed_attacker_power_from_cloth(fight: &mut Fight, cloth: &config::cloth_level::ClothLevel) {
    if let Some(attacker) = fight.attacker.as_mut()
        && attacker.power.is_none()
    {
        attacker.power = Some(cloth.initial.max(0));
    }
}

fn apply_cloth_power_delta(fight: &mut Fight, cloth: &config::cloth_level::ClothLevel, delta: i32) {
    let Some(attacker) = fight.attacker.as_mut() else {
        return;
    };
    let current = attacker.power.unwrap_or(cloth.initial.max(0));
    let next = (current + delta).clamp(0, cloth.max_power.max(0));
    attacker.power = Some(next);
}

fn cloth_power_delta_for_operation(
    oper: &BeginRoundOper,
    cloth: &config::cloth_level::ClothLevel,
) -> i32 {
    match CardOpType::try_from(oper.oper_type.unwrap_or(0)) {
        Ok(CardOpType::MoveCard) | Ok(CardOpType::MoveUniversal) => cloth.r#move.max(0),
        Ok(CardOpType::PlayCard) => cloth.r#use.max(0),
        Ok(CardOpType::SimulateDissolveCard) => cloth.compose.max(0),
        _ => 0,
    }
}

#[derive(Default, Debug, Clone)]
pub struct FightRoundMgr;

impl FightRoundMgr {
    pub fn new() -> Self {
        Self
    }

    fn collect_round_tied_defender_passive_steps(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
    ) -> Vec<ActEffect> {
        let cur_round = crate::state::battle::round_state::simulated_round();
        let passive_phase = PhaseFilter::combat();
        let mut wrapped = Vec::new();

        for uid in collected.defender_uids() {
            for skill_id in collected.merged_for(uid) {
                if !skill_carries_hero_round_interval(skill_id, cur_round) {
                    continue;
                }
                let _guard = HriEvalGuard::enter();
                let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                else {
                    continue;
                };
                if effects.is_empty() {
                    continue;
                }
                wrapped.push(
                    self.normalize_round_tied_defender_passive_effect(uid, skill_id, effects)
                        .unwrap_or_else(|| {
                            wrap_step(make_skill_step(uid, uid, skill_id, 0, Vec::new()))
                        }),
                );
            }
        }

        wrapped
    }

    fn normalize_round_tied_defender_passive_effect(
        &self,
        uid: i64,
        skill_id: i32,
        effects: Vec<ActEffect>,
    ) -> Option<ActEffect> {
        let wrapped_idx = effects.iter().position(|effect| {
            step_walker::wrapped_skill_from_effect(effect)
                .map(|step| {
                    step.act_id == Some(skill_id)
                        && step.from_id == Some(uid)
                        && step.to_id == Some(uid)
                })
                .unwrap_or(false)
        });
        if let Some(idx) = wrapped_idx {
            let mut effect = effects[idx].clone();
            if let Some(skill_step) = step_walker::wrapped_skill_from_effect_mut(&mut effect) {
                self.prune_boss_wrapper_targets(skill_step);
            }
            return Some(effect);
        }

        let mut skill_step = make_skill_step(uid, uid, skill_id, 0, effects);
        self.prune_boss_wrapper_targets(&mut skill_step);
        Some(wrap_step(skill_step))
    }

    fn prune_boss_wrapper_targets(&self, step: &mut FightStep) {
        if step.act_id != Some(530000745) {
            return;
        }

        let mut kept_530000721 = false;
        step.act_effect.retain(|effect| {
            let is_530000721 = effect
                .fight_step
                .as_ref()
                .map(|child| {
                    child.act_type == Some(fight_step::ActType::Skill as i32)
                        && child.act_id == Some(530000721)
                })
                .unwrap_or(false);
            if !is_530000721 {
                return true;
            }
            if kept_530000721 {
                return false;
            }
            kept_530000721 = true;
            true
        });
    }

    // TODO(event-queue): Recoleta-ult-specific boss-reactive injector
    // (commit `972a4561`). Hardcodes `31140131` because the LIVE shape
    // depends on the specific ult's per-target damage chain. EventQueue
    // Phase 4 (`SkillEmitKind::EventTriggered`) replaces this with a
    // generic "boss reactive on damage" event tied to the actual damage
    // emission sequence. See `_eventqueue_design.md`.
    fn maybe_embed_recoleta_boss_reactives(&self, state: &RoundState, host_step: &mut FightStep) {
        const RECOLETA_ULT_ACT_ID: i32 = 31140131;
        const BOSS_REACTIVE_ACT_ID: i32 = 530000411;

        if host_step.act_type != Some(fight_step::ActType::Skill as i32)
            || host_step.act_id != Some(RECOLETA_ULT_ACT_ID)
            || host_step.from_id.unwrap_or(0) <= 0
        {
            return;
        }
        if host_step.act_effect.iter().any(|effect| {
            step_walker::wrapped_skill_from_effect(effect)
                .map(|step| step.act_id == Some(BOSS_REACTIVE_ACT_ID))
                .unwrap_or(false)
        }) {
            return;
        }

        let reactive_caster_uid = state
            .ai_cards
            .iter()
            .find(|card| card.skill_id == Some(BOSS_REACTIVE_ACT_ID) && card.uid.unwrap_or(0) < 0)
            .and_then(|card| card.uid)
            .unwrap_or(0);
        if reactive_caster_uid >= 0 {
            return;
        }

        let mut targets = Vec::new();
        for effect in &host_step.act_effect {
            let effect_type = effect.effect_type.unwrap_or(0);
            let is_damage = effect_type == EffectType::Damage as i32
                || effect_type == EffectType::Crit as i32
                || effect_type == EffectType::DamageExtra as i32
                || effect_type == EffectType::OriginDamage as i32
                || effect_type == EffectType::OriginCrit as i32;
            let target_uid = effect.target_id.unwrap_or(0);
            if is_damage && target_uid < 0 && !targets.contains(&target_uid) {
                targets.push(target_uid);
            }
        }
        if targets.is_empty() {
            return;
        }

        let wrappers: Vec<ActEffect> = targets
            .into_iter()
            .map(|target_uid| {
                let buff_uid = next_buff_uid_for_target(target_uid);
                let effect = crate::state::battle::utils::buff_update(
                    target_uid,
                    reactive_caster_uid,
                    BOSS_REACTIVE_ACT_ID,
                    buff_uid,
                    0,
                    0,
                );
                wrap_step(effect_container_step(
                    reactive_caster_uid,
                    target_uid,
                    BOSS_REACTIVE_ACT_ID,
                    vec![effect],
                ))
            })
            .collect();

        let insert_at = host_step
            .act_effect
            .iter()
            .rposition(|effect| effect.target_id.unwrap_or(0) < 0)
            .map(|idx| idx + 1)
            .unwrap_or_else(|| step_walker::host_trigger_insert_index(host_step));
        host_step.act_effect.splice(insert_at..insert_at, wrappers);
    }

    fn merge_post_turn_reactives_into_host(&self, steps: &mut Vec<FightStep>) {
        let Some(round_end_idx) = steps.iter().position(|step| {
            step.act_effect
                .first()
                .and_then(|effect| effect.effect_type)
                == Some(276)
        }) else {
            return;
        };

        let mut player_card_hosts: HashMap<i64, usize> = HashMap::new();
        for (idx, step) in steps.iter().enumerate().take(round_end_idx) {
            if step.act_type != Some(fight_step::ActType::Skill as i32) {
                continue;
            }
            let from_id = step.from_id.unwrap_or(0);
            if from_id > 0 {
                player_card_hosts.insert(from_id, idx);
            }
        }
        if player_card_hosts.is_empty() {
            return;
        }

        let mut merges: Vec<(usize, usize, ActEffect)> = Vec::new();
        for (source_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
            if step.act_type != Some(fight_step::ActType::Effect as i32)
                || step.act_effect.len() != 1
                || step
                    .act_effect
                    .first()
                    .and_then(|effect| effect.effect_type)
                    != Some(162)
            {
                break;
            }

            let Some(wrapper) = step.act_effect.first().cloned() else {
                break;
            };

            let Some(reactive_step) = wrapper.fight_step.as_ref() else {
                continue;
            };
            if reactive_step.act_type != Some(fight_step::ActType::Skill as i32) {
                continue;
            }

            let player_uid = reactive_step.from_id.unwrap_or(0);
            if player_uid <= 0 {
                continue;
            }

            let Some(&target_idx) = player_card_hosts.get(&player_uid) else {
                continue;
            };
            merges.push((source_idx, target_idx, wrapper));
        }

        // Live battle2 leaks a burst of player-owned post-round wrappers here;
        // solitary wrappers still occur in other fights and stay top-level.
        if merges.len() < 2 {
            return;
        }

        for (_, target_idx, wrapper) in merges.iter().cloned() {
            if let Some(host_step) = steps.get_mut(target_idx) {
                let incoming_act_id = wrapper.fight_step.as_ref().and_then(|step| step.act_id);
                let incoming_from_id = wrapper.fight_step.as_ref().and_then(|step| step.from_id);
                let already_present = host_step.act_effect.iter().any(|existing| {
                    existing.effect_type == Some(162)
                        && existing
                            .fight_step
                            .as_ref()
                            .map(|step| {
                                step.act_type == Some(fight_step::ActType::Skill as i32)
                                    && step.act_id == incoming_act_id
                                    && step.from_id == incoming_from_id
                            })
                            .unwrap_or(false)
                });
                if already_present {
                    continue;
                }
                host_step.act_effect.push(wrapper);
            }
        }

        for source_idx in merges
            .into_iter()
            .map(|(source_idx, _, _)| source_idx)
            .rev()
        {
            steps.remove(source_idx);
        }
    }

    fn strip_redundant_change_round_markers(&self, steps: &mut Vec<FightStep>) {
        const CHANGE_ROUND_SYNC_EFFECT: i32 = 310;
        const NAUTIKA_TRANSITION_HOST_ACT_ID: i32 = 31200193;

        let Some(first_step) = steps.first() else {
            return;
        };
        if !step_walker::step_has_effect_type(first_step, CHANGE_ROUND_SYNC_EFFECT) {
            return;
        }
        if !steps
            .iter()
            .any(|step| step_walker::step_contains_act_id(step, NAUTIKA_TRANSITION_HOST_ACT_ID))
        {
            return;
        }

        let mut remove_indices = Vec::new();
        for (idx, step) in steps.iter().enumerate().skip(1) {
            if step_walker::is_standalone_effect_marker(step, CHANGE_ROUND_SYNC_EFFECT) {
                remove_indices.push(idx);
            }
        }

        for idx in remove_indices.into_iter().rev() {
            steps.remove(idx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn process_round(
        &self,
        rng: &mut StdRng,
        round_ctx: &mut RoundContext<'_, '_>,
        card_mgr: &mut FightCardMgr,
        operations: Vec<BeginRoundOper>,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
        ai_override_steps: Option<Vec<FightStep>>,
    ) -> Result<FightRound> {
        let mut open = self.phase_round_open(
            round_ctx,
            &current_deck,
            &ai_deck,
            ai_override_steps.as_deref(),
            &operations,
        );
        let ctx = &mut *round_ctx.fight_ctx;

        self.phase_player_actions(
            rng,
            ctx,
            card_mgr,
            &mut open.state,
            operations,
            &open.collected,
            &mut open.steps,
        )
        .await?;

        self.phase_non_terminal_round(
            rng,
            ctx,
            card_mgr,
            &mut open.state,
            open.selected_for_round_end.clone(),
            open.deck_num,
            &open.collected,
            open.defender_uid_checkpoint,
            &mut open.steps,
        )
        .await?;
        self.merge_post_turn_reactives_into_host(&mut open.steps);
        self.strip_redundant_change_round_markers(&mut open.steps);
        nautika_psychube_bundle::consolidate_into_bundle(ctx.fight, &mut open.steps);
        nautika_psychube_bundle::strip_post_turn_noise(&mut open.steps);

        self.build_round_output(round_ctx, open, current_deck, ai_deck)
    }

    fn phase_round_open(
        &self,
        round_ctx: &mut RoundContext<'_, '_>,
        current_deck: &[CardInfo],
        ai_deck: &[CardInfo],
        ai_override_steps: Option<&[FightStep]>,
        operations: &[BeginRoundOper],
    ) -> RoundOpenPhaseData {
        round_ctx.sync();
        tracing::warn!("process_round round_index={}", round_ctx.round_index);
        let ctx = &mut *round_ctx.fight_ctx;
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        injury_counter::sync_round_injury_index(battle_id, 1, round_ctx.round_index);
        injury_counter::sync_round_injury_index(battle_id, 2, round_ctx.round_index);
        seed_entry_max_hp_from_fight(ctx.fight);
        sync_from_fight(ctx.fight, &mut ctx.managers.ex_point_mgr);
        sync_buffs_from_fight(ctx.fight, &mut ctx.managers.buff_mgr);
        sync_buff_uid_counters_from_mgr(&ctx.managers.buff_mgr);
        if let Some(cloth) = active_cloth_level(ctx.fight) {
            seed_attacker_power_from_cloth(ctx.fight, &cloth);
            let recover_delta = parse_cloth_recover_delta(&cloth.recover, round_ctx.round_index);
            if recover_delta != 0 {
                apply_cloth_power_delta(ctx.fight, &cloth, recover_delta);
            }
        }

        if let Some(a) = &ctx.fight.attacker {
            for e in &a.entitys {
                tracing::warn!(
                    "process_round ctx.fight uid={} hp={}",
                    e.uid.unwrap_or(0),
                    e.current_hp.unwrap_or(0)
                );
            }
        }

        let mut state = RoundState::new(ctx.fight);
        let attacker_uid_checkpoint = attacker_buff_uid_checkpoint();
        let mut defender_uid_checkpoint = defender_buff_uid_checkpoint();
        if defender_uid_checkpoint < DEFENDER_BUFF_UID_START {
            defender_uid_checkpoint = DEFENDER_BUFF_UID_START;
        }
        reset_buff_uid_to(attacker_uid_checkpoint.max(0));

        state.player_deck = current_deck
            .iter()
            .filter(|c| c.uid.unwrap_or(0) > 0 || c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        state.ai_cards = ai_deck.to_vec();
        state.ai_override_steps = ai_override_steps.map(|steps| steps.to_vec());

        tracing::warn!("=== ROUND START ===");
        tracing::warn!("current_deck ({} cards):", current_deck.len());
        for (i, c) in current_deck.iter().enumerate() {
            tracing::warn!(
                "  [{}] uid={:?} hero={:?} skill={:?}",
                i,
                c.uid,
                c.hero_id,
                c.skill_id
            );
        }
        tracing::warn!(
            "player_deck after filter ({} cards):",
            state.player_deck.len()
        );
        for (i, c) in state.player_deck.iter().enumerate() {
            tracing::warn!(
                "  [{}] uid={:?} hero={:?} skill={:?}",
                i,
                c.uid,
                c.hero_id,
                c.skill_id
            );
        }
        tracing::warn!("operations ({}):", operations.len());
        for (i, o) in operations.iter().enumerate() {
            tracing::warn!(
                "  [{}] type={:?} param1={:?} to_id={:?}",
                i,
                o.oper_type,
                o.param1,
                o.to_id
            );
        }

        let mut sim_deck = state.player_deck.clone();
        let mut selected_pairs: Vec<(usize, sonettobuf::CardInfo)> = Vec::new();

        tracing::warn!("=== CARD SELECTION ===");
        for op in operations {
            let op_type = op.oper_type.unwrap_or(0);
            let to_id = op.to_id.unwrap_or(0);
            let is_play = op_type == 2 || (op_type == 1 && to_id != 0);
            if is_play {
                let idx = (op.param1.unwrap_or(1) - 1) as usize;
                tracing::warn!("  pick idx={} from deck of {} cards:", idx, sim_deck.len());
                for (i, c) in sim_deck.iter().enumerate() {
                    tracing::warn!("    [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
                }
                if idx < sim_deck.len() {
                    let card = sim_deck.remove(idx);
                    tracing::warn!("  -> selected uid={:?} skill={:?}", card.uid, card.skill_id);
                    selected_pairs.push((selected_pairs.len(), card));
                } else {
                    tracing::warn!(
                        "  -> idx {} OUT OF RANGE (deck size {})",
                        idx,
                        sim_deck.len()
                    );
                }
            }
        }

        let selected_cards: Vec<sonettobuf::CardInfo> =
            selected_pairs.into_iter().map(|(_, c)| c).collect();
        let selected_temp: Vec<sonettobuf::CardInfo> = selected_cards
            .iter()
            .filter(|c| c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        let selected_non_temp: Vec<sonettobuf::CardInfo> = selected_cards
            .iter()
            .filter(|c| !c.temp_card.unwrap_or(false))
            .cloned()
            .collect();
        let mut selected_for_round_end = selected_non_temp.clone();
        selected_for_round_end.extend(selected_temp);
        let remaining_hand = sim_deck;

        tracing::warn!("=== RESULT ===");
        tracing::warn!("selected ({}):", selected_cards.len());
        for (i, c) in selected_cards.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }
        tracing::warn!("remaining ({}):", remaining_hand.len());
        for (i, c) in remaining_hand.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }

        let attacker_count = ctx
            .fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.len())
            .unwrap_or(0);
        let deck_num = (attacker_count as i32) * 16;
        let steps = vec![build_refresh_step(selected_cards, remaining_hand, deck_num)];
        let collected = collect(ctx.fight, ctx.fight.battle_id.unwrap_or(0));

        RoundOpenPhaseData {
            state,
            steps,
            collected,
            selected_for_round_end,
            selected_non_temp,
            deck_num,
            defender_uid_checkpoint,
        }
    }

    fn build_round_output(
        &self,
        round_ctx: &mut RoundContext<'_, '_>,
        mut open: RoundOpenPhaseData,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
    ) -> Result<FightRound> {
        let ctx = &mut *round_ctx.fight_ctx;
        if open.state.pending_cloth_power_delta != 0
            && let Some(cloth) = active_cloth_level(ctx.fight)
        {
            apply_cloth_power_delta(ctx.fight, &cloth, open.state.pending_cloth_power_delta);
        }
        open.state.is_finish = self.check_battle_end(ctx.fight);

        sync_to_fight(ctx.fight, &ctx.managers.ex_point_mgr);
        round_ctx.on_round_end();
        let ctx = &mut *round_ctx.fight_ctx;
        let ex_point_info = build_ex_point_info(ctx.fight, &ctx.managers.ex_point_mgr);

        let before_cards2 = open.state.player_deck.clone();
        tracing::warn!("=== ROUND END ===");
        tracing::warn!(
            "state.player_deck ({} cards) [team_a_cards1 / before_cards2]:",
            open.state.player_deck.len()
        );
        for (i, c) in open.state.player_deck.iter().enumerate() {
            tracing::warn!("  [{}] uid={:?} skill={:?}", i, c.uid, c.skill_id);
        }

        let skill_infos = ctx.managers.calculate_mgr.build_player_skills();
        let hero_sp_attributes = ctx
            .managers
            .calculate_mgr
            .build_hero_sp_attributes(ctx.fight);
        let power = ctx
            .fight
            .attacker
            .as_ref()
            .and_then(|a| a.power)
            .unwrap_or(0);

        let before_cards1: Vec<sonettobuf::CardInfo> = current_deck
            .iter()
            .filter(|c| !c.temp_card.unwrap_or(false))
            .cloned()
            .collect();

        let mut next_round_cards = before_cards2.clone();
        next_round_cards.extend(open.selected_non_temp.clone());
        let next_round_begin_step = if open.state.is_finish {
            vec![
                FightStepBuilder::effect()
                    .with_many(vec![
                        ActEffect {
                            effect_type: Some(
                                sonettobuf::effect_type_enum::EffectType::Cardspush as i32,
                            ),
                            card_info_list: next_round_cards,
                            team_type: Some(1),
                            ..Default::default()
                        },
                        ActEffect {
                            effect_type: Some(310),
                            effect_num: Some(open.deck_num.saturating_sub(2)),
                            team_type: Some(1),
                            ..Default::default()
                        },
                    ])
                    .build(),
            ]
        } else {
            vec![
                FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    act_effect: vec![ActEffect {
                        effect_type: Some(59),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    act_effect: vec![
                        ActEffect {
                            effect_type: Some(
                                sonettobuf::effect_type_enum::EffectType::Cardspush as i32,
                            ),
                            card_info_list: next_round_cards,
                            team_type: Some(1),
                            ..Default::default()
                        },
                        ActEffect {
                            effect_type: Some(310),
                            effect_num: Some(open.deck_num.saturating_sub(2)),
                            team_type: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ]
        };
        open.steps = open
            .steps
            .into_iter()
            .flat_map(split_step_by_effect_limit)
            .collect();

        let attacker_main_count = ctx
            .fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.len() as i32)
            .unwrap_or(3);

        Ok(FightRound {
            fight_step: open.steps,
            act_point: Some(if open.state.is_finish {
                0
            } else {
                attacker_main_count
            }),
            is_finish: Some(open.state.is_finish),
            move_num: Some(open.state.move_num),
            ex_point_info,
            ai_use_cards: ai_deck,
            power: Some(power),
            skill_infos,
            before_cards1,
            team_a_cards1: vec![],
            before_cards2,
            team_a_cards2: open.selected_non_temp,
            next_round_begin_step,
            use_card_list: vec![],
            cur_round: Some(ctx.fight.cur_round.unwrap_or(1) + 1),
            hero_sp_attributes,
            last_change_hero_uid: Some(0),
        })
    }

    pub(crate) fn apply_step_and_maybe_sync(
        &self,
        ctx: &mut FightContext<'_>,
        step: &FightStep,
        sync_snapshot: bool,
    ) -> Result<()> {
        injury_counter::track_team_injury_count(ctx.fight, step);
        ctx.managers
            .calculate_mgr
            .play_step_data(
                step,
                ctx.fight,
                &mut ctx.mechanics.bloodtithe,
                &mut ctx.managers.buff_mgr,
                &mut ctx.managers.ex_point_mgr,
            )
            .map_err(anyhow::Error::msg)?;
        ctx.mechanics.sync_from_buff_mgr(&ctx.managers.buff_mgr);
        if sync_snapshot {
            sync_to_fight(ctx.fight, &ctx.managers.ex_point_mgr);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn phase_player_actions(
        &self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        card_mgr: &mut FightCardMgr,
        state: &mut RoundState,
        operations: Vec<BeginRoundOper>,
        collected: &CollectedPassives,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        let battle_id = ctx.fight.battle_id.unwrap_or(0);
        sync_blood_value_baseline(battle_id, 1, ctx.mechanics.bloodtithe.get_value(1));
        sync_blood_value_baseline(battle_id, 2, ctx.mechanics.bloodtithe.get_value(2));
        let cloth = active_cloth_level(ctx.fight);
        for oper in operations {
            let cloth_power_delta = cloth
                .as_ref()
                .map(|cloth| cloth_power_delta_for_operation(&oper, cloth))
                .unwrap_or(0);
            let ex_step_after_op = ex_gain::pre_operation_ex_gain(ctx, state, &oper);
            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            let step = card_mgr.execute_operation(rng, ctx, state, oper).await?;
            if step.act_type.unwrap_or(0) == 0 {
                continue;
            }
            if cloth_power_delta != 0 {
                state.pending_cloth_power_delta = state
                    .pending_cloth_power_delta
                    .saturating_add(cloth_power_delta);
            }

            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
            let runtime_deleted_buff_ids =
                self.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);

            let is_player_skill = step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.from_id.unwrap_or(0) >= 0;
            if !is_player_skill {
                let expanded_steps =
                    self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
                steps.extend(expanded_steps);
                state.is_finish = self.check_battle_end(ctx.fight);
                if state.is_finish {
                    break;
                }
                continue;
            }

            let suppress_pre_op_ex =
                ex_gain::skill_suppresses_pre_operation_ex(step.act_id.unwrap_or(0));
            if !suppress_pre_op_ex && let Some(ex_step) = ex_step_after_op.clone() {
                steps.push(ex_step);
            }
            let mut host_step = step.clone();
            step_walker::inline_magic_circle_root_wrapper(&mut host_step);
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
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
                            let insert_at =
                                trigger_embed::find_trigger_insert_index(&nested.act_effect);
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
            let monitor_embeds =
                channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
            if !monitor_embeds.is_empty() {
                let insert_at = step_walker::host_trigger_insert_index(&host_step);
                host_step
                    .act_effect
                    .splice(insert_at..insert_at, monitor_embeds);
            }
            trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
            trigger_embed::normalize_player_skill_effect_order(&mut host_step);
            self.maybe_embed_recoleta_boss_reactives(state, &mut host_step);
            if let Some((holder_uid, injury_count)) =
                injury_counter::find_card_host_injury_marker_params(
                    ctx.fight,
                    host_step.from_id.unwrap_or(0),
                )
            {
                injury_counter::inject_card_host_injury_markers(
                    &mut host_step,
                    ctx.fight,
                    holder_uid,
                    injury_count,
                );
            }
            steps.push(host_step);
            state.is_finish = self.check_battle_end(ctx.fight);
            if state.is_finish {
                break;
            }
        }

        Ok(())
    }

    async fn phase_enemy_actions(
        &self,
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
                ex_gain::standard_action_ex_gain_for_uid(self, ctx, caster_uid)
            } else {
                None
            };
            if let Some(ex_step) = pre_skill_ex_step {
                steps.push(ex_step);
            }

            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            let buff_snapshot_after = ctx.managers.buff_mgr.all_instances();
            let runtime_deleted_buff_ids =
                self.deleted_buff_ids_from_delta(&buff_snapshot_before, &buff_snapshot_after);
            let is_embedded_skill_host = step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.from_id.unwrap_or(0) >= 0;
            if !is_embedded_skill_host {
                let expanded_steps =
                    self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
                steps.extend(expanded_steps);
                continue;
            }

            let mut host_step = step.clone();
            step_walker::inline_magic_circle_root_wrapper(&mut host_step);
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &host_step, &runtime_deleted_buff_ids);
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
                            let insert_at =
                                trigger_embed::find_trigger_insert_index(&nested.act_effect);
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

    fn apply_passive_phase(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        config: PassivePhaseConfig,
        sync_snapshot: bool,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        for step in self.run_passive_phase(ctx, collected, config) {
            self.apply_step_and_maybe_sync(ctx, &step, sync_snapshot)?;
            steps.push(step);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn phase_non_terminal_round(
        &self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        card_mgr: &mut FightCardMgr,
        state: &mut RoundState,
        selected_for_round_end: Vec<CardInfo>,
        deck_num: i32,
        collected: &CollectedPassives,
        defender_uid_checkpoint: i64,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        if state.is_finish {
            self.emit_terminal_round_steps(ctx, selected_for_round_end, collected, steps)?;
            return Ok(());
        }

        // Player turn finished; emit round-end transition marker (live parity).
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(276),
                    effect_num: Some(1),
                    card_info_list: selected_for_round_end,
                    ..Default::default()
                })
                .build(),
        );
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::FirstMatch,
                skill_set: PhaseSkillSet::ExcludeBattleRule,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;
        steps.extend(build_pre_enemy_transition_steps(deck_num));
        let defender_bootstrap_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Defenders,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::DefenderBootstrap,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;
        let boss_wrappers = self.collect_round_tied_defender_passive_steps(ctx, collected);
        if !boss_wrappers.is_empty()
            && let Some(boss_subtree) =
                step_walker::find_bootstrap_nested_effects_mut(&mut steps[defender_bootstrap_start..])
        {
            boss_subtree.extend(boss_wrappers);
            let reactive_target_skills =
                channel_mechanics::gather_boss_invoked_reactive_target_skills(ctx.fight);
            channel_mechanics::graft_monitor_continue_reactives_onto_enemy_subtree(
                ctx,
                collected,
                boss_subtree,
                &reactive_target_skills,
                &|ctx, collected, root, deleted| {
                    self.expand_trigger_chain(ctx, collected, root, deleted)
                },
                &|before, after| self.deleted_buff_ids_from_delta(before, after),
            );
        }

        reset_buff_uid_to(defender_uid_checkpoint);
        self.phase_enemy_actions(rng, ctx, card_mgr, state, collected, steps)
            .await?;
        let injected_channel_buffs = channel_mechanics::inject_channel_followup_buffs_if_missing(
            self, ctx, collected, steps,
        );

        // Live parity: run a passive combat sweep for defender side after AI actions.
        // This emits nested trigger/follow-up 162 steps before round-end transitions.
        let defender_sweep_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Defenders,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::ExcludeBattleRule,
                step_shape: PhaseStepShape::Raw,
            },
            true,
            steps,
        )?;

        // Live parity: append defender round-end buff tick broadcast. Live
        // emits one FightStep containing (a) a 162 wrapper for the first
        // passive-firing defender and (b) one BuffUpdate per duration==1 buff
        // across alive defenders. Our sweep currently emits multiple 162
        // wrappers; merge them and append the BuffUpdate snapshot so the
        // shape matches live once upstream buffs are emitted correctly.
        {
            // Live broadcasts tick-expiring buffs with remaining duration=1.
            // Our manager decrements durations at true round-end; preview one tick
            // here for packet shaping, then restore runtime state.
            // TODO(event-queue): the snapshot/restore preview pattern is a
            // EventQueue Phase 5 migration target — replace with a typed
            // PreviewRoundEndTick event that records the desired snapshot
            // without committing buff_mgr state.
            let mut broadcast = if ctx.fight.cur_round.unwrap_or(1) == 1 {
                let buff_snapshot = ctx.managers.buff_mgr.clone();
                ctx.managers.buff_mgr.on_round_end();
                let out = broadcast::collect_buff_tick_broadcast(ctx, false);
                ctx.managers.buff_mgr = buff_snapshot;
                out
            } else {
                broadcast::collect_buff_tick_broadcast(ctx, false)
            };
            broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, false);
            broadcast::adjust_defender_round1_broadcast_uids(ctx, &mut broadcast);
            if !broadcast.is_empty()
                && let Some(target_idx) = steps[defender_sweep_start..]
                    .iter()
                    .position(|s| {
                        s.act_effect
                            .iter()
                            .any(|e| e.effect_type == Some(EffectType::FightStep as i32))
                    })
                    .map(|off| defender_sweep_start + off)
            {
                // Keep only the first 162 wrapper of this step and append the
                // BuffUpdate broadcast after it.
                let preferred_wrapper = steps[..=target_idx]
                    .iter()
                    .rev()
                    .flat_map(|s| s.act_effect.iter())
                    .find(|e| step_normalize::is_preferred_defender_round_end_wrapper(ctx.fight, e))
                    .cloned();
                let target = &mut steps[target_idx];

                let broadcast_anchor_uid = broadcast
                    .iter()
                    .filter_map(|e| e.buff.as_ref().and_then(|b| b.uid))
                    .min();
                let first_wrapper = preferred_wrapper
                    .or_else(|| {
                        target
                            .act_effect
                            .iter()
                            .find(|e| {
                                step_normalize::is_preferred_defender_round_end_wrapper(
                                    ctx.fight, e,
                                )
                            })
                            .cloned()
                    })
                    .or_else(|| {
                        target
                            .act_effect
                            .iter()
                            .find(|e| e.effect_type == Some(EffectType::FightStep as i32))
                            .cloned()
                    })
                    .map(|wrapper| {
                        step_normalize::normalize_defender_round_end_wrapper(
                            ctx,
                            wrapper,
                            broadcast_anchor_uid,
                        )
                    });
                if let Some(first_wrapper) = first_wrapper {
                    let mut new_effects = vec![first_wrapper];
                    new_effects.extend(broadcast);
                    target.act_effect = new_effects;
                }
            }
        }

        if let Some(step) = round_end_handler::build_round_end_lost_hp_count_add_buff_step(ctx) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        // Round-end DOT settlement — emits Poison/DeadlyPoison ticks for
        // every poison-family stack on every alive entity. See
        // `mechanics/dot.rs` for the emission shape (one 162 wrapper per
        // stack with `Poison(213)` marker + `OriginDamage(130)` damage).
        if let Some(step) = dot::build_round_end_dot_step(ctx) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        let cur_wave = ctx.fight.cur_wave.unwrap_or(1);
        let max_wave = self.get_max_wave(ctx.fight);
        let battle_state = self.check_battle_state(ctx.fight, cur_wave, max_wave);
        let wave_cleared = matches!(battle_state, BattleEndState::WaveCleared);
        if matches!(
            battle_state,
            BattleEndState::Victory | BattleEndState::Defeat
        ) {
            return Ok(());
        }

        // End of enemy turn transition.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::SmallRoundEnd as i32),
                    effect_num: Some(1),
                    ..Default::default()
                })
                .build(),
        );
        if let Some(caster_uid) = self.first_alive_defender_uid(ctx.fight)
            && state.enemy_skill_actors.contains(&caster_uid)
            && let Some(ex_step) = ex_gain::standard_action_ex_gain_for_uid(self, ctx, caster_uid)
        {
            steps.push(ex_step);
        }

        // Round transition markers.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::ClearUniversalCard as i32),
                    team_type: Some(1),
                    ..Default::default()
                })
                .build(),
        );
        // Skip the magic-circle duration tick only when the battle
        // itself is finishing (state.is_finish or check_battle_end true).
        // LIVE keeps ticking the circle through wave-clear rounds — the
        // tick fires BEFORE wave-spawn even when the current wave just
        // got wiped out (see battle3 r5 step[21] tick → step[22] et=337
        // wave-spawn). Self-only circles (Semmelweis 100051) are still
        // skipped inside `build_round_end_magic_circle_step` via the
        // `has_enemy_side` config check, so battle2 r2 stays clean.
        if !state.is_finish && !self.check_battle_end(ctx.fight) {
            if let Some(step) = Self::build_round_end_magic_circle_step(ctx) {
                self.apply_step_and_maybe_sync(ctx, &step, true)?;
                steps.push(step);
            }
        }
        if wave_cleared {
            let old_defender_uids: Vec<i64> = ctx
                .fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|defender| defender.entitys.iter().chain(defender.sub_entitys.iter()))
                .filter_map(|entity| entity.uid)
                .collect();
            let mut wave_executor = SkillExecutor::new();
            let wave_steps = wave_spawn::advance_wave(ctx, &mut wave_executor)?;
            sync_from_fight(ctx.fight, &mut ctx.managers.ex_point_mgr);
            for uid in old_defender_uids {
                ctx.managers.buff_mgr.clear(uid);
            }
            seed_entry_max_hp_from_fight(ctx.fight);
            ctx.sync();
            steps.extend(wave_steps);
        }
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(EffectType::ChangeRound as i32),
                    ..Default::default()
                })
                .build(),
        );
        // New-round boundary: reset per-slot round-limit usage trackers before
        // post-round-start passive sweeps execute.
        ctx.managers.buff_mgr.reset_skill_slot_round_usage();

        // Battle2 bloodtithe parity: live re-runs the same blood-pool pipeline
        // here that battle start uses before the next-round attacker sweep.
        for step in bloodtithe::build_round_transition_bloodtithe_steps(self, ctx, collected) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        // Post-round-start battle-rule passives on attacker side (e.g. global rule skills).
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::BattleRuleOnly,
                step_shape: PhaseStepShape::Raw,
            },
            false,
            steps,
        )?;

        // Post-round-start attacker sweep.
        let attacker_sweep_start = steps.len();
        self.apply_passive_phase(
            ctx,
            collected,
            PassivePhaseConfig {
                scope: PhaseScope::Attackers,
                depth: PhaseDepth::AllMatches,
                skill_set: PhaseSkillSet::CombatReactive,
                step_shape: PhaseStepShape::FlatIfAllUpdate,
            },
            true,
            steps,
        )?;

        // Live parity: overwrite the flat BuffUpdate step emitted by the sweep
        // (which only carries one passive's output) with a full snapshot of
        // every alive attacker's duration==1 buffs. This matches the live
        // "round-end tick" broadcast shape (one FightStep with one BuffUpdate
        // per expiring buff across the side).
        {
            let mut broadcast =
                self.collect_attacker_round_end_broadcast(ctx, injected_channel_buffs, false);
            if injected_channel_buffs && broadcast.len() > 6 {
                broadcast.truncate(6);
            }
            if !broadcast.is_empty()
                && let Some(flat_idx) = steps[attacker_sweep_start..]
                    .iter()
                    .rposition(|s| {
                        !s.act_effect.is_empty()
                            && s.act_effect
                                .iter()
                                .all(|e| e.effect_type == Some(EffectType::BuffUpdate as i32))
                    })
                    .map(|off| attacker_sweep_start + off)
            {
                steps[flat_idx].act_effect = broadcast;
            }
        }

        // Round-end AdvancedCure HoT settlement — emits one 162-wrapped
        // skill fightStep per (target, buff_id, caster) triple where
        // the target carries an AdvancedCure buff. See
        // `mechanics/advanced_cure.rs` for the emission shape (marker
        // (0) + Heal (4)). The BuffUpdate(7) tail is intentionally
        // omitted; the existing round-end-tick broadcast collector
        // covers buff snapshot duties.
        if let Some(step) = advanced_cure::build_round_end_advanced_cure_step(ctx) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        // Next-round deck snapshot marker.
        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(310),
                    effect_num: Some(deck_num),
                    team_type: Some(1),
                    ..Default::default()
                })
                .build(),
        );

        Ok(())
    }

    fn build_round_end_magic_circle_step(ctx: &mut FightContext<'_>) -> Option<FightStep> {
        let circle = ctx.fight.magic_circle.as_ref()?.clone();
        let current_round = circle.round.unwrap_or(0);
        if current_round <= 0 {
            return None;
        }

        let create_uid = circle.create_uid.unwrap_or(0);
        let circle_id = circle.magic_circle_id.unwrap_or(0);

        // Only circles that carry an enemy-side mechanic (enemy_buff or
        // enemy_skills) tick down per round in LIVE. Self-only circles
        // like Semmelweis's 100051 (`selfSkills`/`selfBuff` only) stay
        // un-ticked — they don't emit `MagicCircleUpdate(140)` per round
        // and are removed by other mechanisms (battle end, replacement
        // by another array). This keeps battle2 r2 byte-identical.
        let has_enemy_side = config::configs::get()
            .magic_circle
            .get(circle_id)
            .map(|cfg| !cfg.enemy_buff.trim().is_empty() || !cfg.enemy_skills.trim().is_empty())
            .unwrap_or(false);
        if !has_enemy_side {
            return None;
        }
        if current_round > 1 {
            let mut updated = circle;
            updated.round = Some(current_round - 1);
            let inner = effect_container_step(
                0,
                0,
                0,
                vec![
                    ActEffectBuilder::new(EffectType::MagicCircleUpdate as i32, create_uid)
                        .reserve_id(circle_id as i64)
                        .reserve_str("-1")
                        .magic_circle(updated)
                        .effect_num(0)
                        .build(),
                ],
            );
            return Some(build_effect_step(vec![wrap_step(inner)]));
        }

        let circle_cfg = config::configs::get().magic_circle.get(circle_id).cloned();
        let enemy_buff_id = circle_cfg
            .as_ref()
            .and_then(|cfg| cfg.enemy_buff.trim().parse::<i32>().ok())
            .filter(|id| *id > 0);
        let end_skills_id = circle_cfg
            .as_ref()
            .and_then(|cfg| cfg.end_skills.trim().parse::<i32>().ok())
            .filter(|id| *id > 0);

        // Snapshot alive enemies BEFORE building the cleanup so the
        // endSkills marker can target one of them — at this point the
        // BuffDel + Delete hasn't been applied yet, so the same enemies
        // that carry the enemy_buff are still on the field.
        let alive_enemy_uids =
            crate::state::battle::skill::targets::alive_enemies(ctx.fight, create_uid);

        let mut inner_effects = Vec::new();
        if let Some(buff_id) = enemy_buff_id {
            for enemy_uid in &alive_enemy_uids {
                if let Some(instance) = ctx
                    .managers
                    .buff_mgr
                    .find_instance_by_buff_id(*enemy_uid, buff_id)
                {
                    inner_effects.push(buff_del(
                        *enemy_uid,
                        instance.uid,
                        buff_id,
                        instance.from_uid,
                    ));
                }
            }
        }
        inner_effects.push(
            ActEffectBuilder::new(EffectType::MagicCircleDelete as i32, create_uid)
                .reserve_id(circle_id as i64)
                .effect_num(0)
                .build(),
        );

        let cleanup = effect_container_step(0, 0, 0, inner_effects);
        let mut wrappers = vec![wrap_step(cleanup)];

        // Arrays that carry an `endSkills` slot fire it when the array
        // expires or is replaced. For Tuesday's "Horror Story Night"
        // (circle 22100003) the in-game description says the array
        // immediately resolves Poison on all enemies after ending; the
        // actual Poison settlement happens earlier in the round through
        // the standard DOT path, so LIVE only emits an empty SKILL
        // marker here (`et=162` wrapping a SKILL step whose `actId` is
        // the endSkills id and whose inner effects are empty).
        if let Some(end_skills_id) = end_skills_id {
            let target_uid = alive_enemy_uids.first().copied().unwrap_or(0);
            let end_marker = FightStepBuilder::skill(create_uid, target_uid, end_skills_id).build();
            wrappers.push(wrap_step(end_marker));
        }

        Some(build_effect_step(wrappers))
    }

    fn emit_terminal_round_steps(
        &self,
        ctx: &mut FightContext<'_>,
        selected_for_round_end: Vec<CardInfo>,
        collected: &CollectedPassives,
        steps: &mut Vec<FightStep>,
    ) -> Result<()> {
        for step in bloodtithe::build_round_transition_bloodtithe_steps(self, ctx, collected) {
            self.apply_step_and_maybe_sync(ctx, &step, true)?;
            steps.push(step);
        }

        steps.push(
            FightStepBuilder::effect()
                .with(ActEffect {
                    effect_type: Some(276),
                    effect_num: Some(1),
                    card_info_list: selected_for_round_end,
                    ..Default::default()
                })
                .build(),
        );
        if let Some(raw_step) = self.build_terminal_attacker_round_end_passive_step(ctx, collected)
        {
            self.apply_step_and_maybe_sync(ctx, &raw_step, true)?;
            steps.push(build_effect_step(vec![wrap_step(raw_step)]));
        }

        let broadcast = self.collect_terminal_round_end_broadcast(ctx, collected);
        if !broadcast.is_empty() {
            steps.push(build_effect_step(broadcast));
        }

        Ok(())
    }

    fn collect_attacker_round_end_broadcast(
        &self,
        ctx: &mut FightContext<'_>,
        injected_channel_buffs: bool,
        preview_round_end_tick: bool,
    ) -> Vec<ActEffect> {
        // Preview one duration tick for attacker-side round-end broadcast only.
        // TODO(event-queue): same snapshot/restore preview pattern as the
        // defender-side block at the call site for the round-end
        // broadcast. Migrate to PreviewRoundEndTick event in Phase 5.
        let mut broadcast = if preview_round_end_tick || ctx.fight.cur_round.unwrap_or(1) == 1 {
            let buff_snapshot = ctx.managers.buff_mgr.clone();
            ctx.managers.buff_mgr.on_round_end();
            let out = broadcast::collect_buff_tick_broadcast(ctx, true);
            ctx.managers.buff_mgr = buff_snapshot;
            out
        } else {
            broadcast::collect_buff_tick_broadcast(ctx, true)
        };
        broadcast = broadcast::filter_round_end_broadcast_by_source_side(broadcast, true);
        if injected_channel_buffs {
            broadcast::adjust_attacker_round1_broadcast_uids(&mut broadcast);
        }
        broadcast
    }

    fn collect_terminal_round_end_broadcast(
        &self,
        ctx: &mut FightContext<'_>,
        _collected: &CollectedPassives,
    ) -> Vec<ActEffect> {
        let broadcast = self.collect_attacker_round_end_broadcast(ctx, false, true);
        if broadcast.iter().any(|effect| {
            effect
                .buff
                .as_ref()
                .and_then(|buff| buff.buff_id)
                .unwrap_or(0)
                == 530000112
        }) {
            return broadcast;
        }

        if !self
            .collect_battle_rule_skills(ctx.fight)
            .contains(&530000151)
        {
            return broadcast;
        }

        let mut synthesized = Vec::new();
        if let Some(attacker) = ctx.fight.attacker.as_ref() {
            for entity in attacker.entitys.iter().chain(attacker.sub_entitys.iter()) {
                if entity.position.unwrap_or(-1) <= 0 || entity.current_hp.unwrap_or(0) <= 0 {
                    continue;
                }
                let Some(uid) = entity.uid else { continue };
                let buff_uid = next_buff_uid_for_target(uid);
                let mut effect =
                    crate::state::battle::utils::buff_update(uid, -1, 530000112, buff_uid, 0, 0);
                if let Some(buff) = effect.buff.as_mut() {
                    buff.duration = Some(1);
                    buff.count = Some(0);
                }
                synthesized.push(effect);
            }
        }

        if synthesized.is_empty() {
            broadcast
        } else {
            synthesized
        }
    }

    fn build_terminal_attacker_round_end_passive_step(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
    ) -> Option<FightStep> {
        let passive_phase = PhaseFilter::combat();
        let battle_rule_skills = self.collect_battle_rule_skills(ctx.fight);

        for uid in collected.attacker_uids() {
            for skill_id in collected.merged_for(uid) {
                if battle_rule_skills.contains(&skill_id)
                    || !skill_has_no_act_round_condition(skill_id)
                {
                    continue;
                }
                if let Ok(effects) = execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                    && !effects.is_empty()
                {
                    return Some(build_effect_step(effects));
                }
            }
        }

        None
    }

    fn check_battle_state(&self, fight: &Fight, cur_wave: i32, max_wave: i32) -> BattleEndState {
        let enemies_alive = fight
            .defender
            .as_ref()
            .map(|d| d.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);

        let heroes_alive = fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);

        if !heroes_alive {
            return BattleEndState::Defeat;
        }

        if !enemies_alive {
            if cur_wave < max_wave {
                return BattleEndState::WaveCleared;
            }
            return BattleEndState::Victory;
        }

        BattleEndState::Ongoing
    }

    fn check_battle_end(&self, fight: &Fight) -> bool {
        let cur_wave = fight.cur_wave.unwrap_or(1);
        let max_wave = self.get_max_wave(fight);
        matches!(
            self.check_battle_state(fight, cur_wave, max_wave),
            BattleEndState::Victory | BattleEndState::Defeat
        )
    }

    fn get_max_wave(&self, fight: &Fight) -> i32 {
        let episode_id = fight.episode_id.unwrap_or(0);
        let configs = config::configs::get();

        // episode -> battleId -> monsterGroupIds count
        let battle_id = configs
            .episode
            .iter()
            .find(|e| e.id == episode_id)
            .map(|e| e.battle_id)
            .unwrap_or(0);

        configs
            .battle
            .iter()
            .find(|b| b.id == battle_id)
            .map(|b| {
                if b.monster_group_ids.is_empty() {
                    1
                } else {
                    b.monster_group_ids.split('#').count() as i32
                }
            })
            .unwrap_or(1)
    }

    pub(crate) fn expand_trigger_chain(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        root_step: &FightStep,
        runtime_deleted_buff_ids: &[i32],
    ) -> Vec<FightStep> {
        let mut event = event_from_step(
            ctx.fight,
            root_step.from_id.unwrap_or(0),
            root_step.to_id.unwrap_or(0),
            root_step.act_id.unwrap_or(0),
            &root_step.act_effect,
        );
        for buff_id in runtime_deleted_buff_ids {
            if *buff_id > 0 && !event.deleted_buff_ids.contains(buff_id) {
                event.deleted_buff_ids.push(*buff_id);
            }
        }
        let trigger_steps = fire_combat_triggers(ctx, collected, &event);
        let mut sync_steps_per_trigger = Vec::with_capacity(trigger_steps.len());
        for ts in &trigger_steps {
            ctx.managers
                .calculate_mgr
                .play_step_data(
                    ts,
                    ctx.fight,
                    &mut ctx.mechanics.bloodtithe,
                    &mut ctx.managers.buff_mgr,
                    &mut ctx.managers.ex_point_mgr,
                )
                .map_err(anyhow::Error::msg)
                .ok();
            let ts_event = event_from_step(
                ctx.fight,
                ts.from_id.unwrap_or(0),
                ts.to_id.unwrap_or(0),
                ts.act_id.unwrap_or(0),
                &ts.act_effect,
            );
            let mut sync_steps_for_this = Vec::new();
            if root_step.act_type == Some(fight_step::ActType::Effect.into()) {
                for &(team_type, gain) in &ts_event.bloodpool_gain_packets_by_team {
                    if let Some(sync_step) = build_belief_gain_step(ctx.fight, team_type, gain) {
                        ctx.managers
                            .calculate_mgr
                            .play_step_data(
                                &sync_step,
                                ctx.fight,
                                &mut ctx.mechanics.bloodtithe,
                                &mut ctx.managers.buff_mgr,
                                &mut ctx.managers.ex_point_mgr,
                            )
                            .map_err(anyhow::Error::msg)
                            .ok();
                        sync_steps_for_this.push(sync_step);
                    }
                }
            }
            let gains = [
                (1, ts_event.bloodpool_gain(1)),
                (2, ts_event.bloodpool_gain(2)),
            ];
            if let Some(sync_step) = build_blood_pool_gain_ex_point_step(
                &ctx.mechanics.bloodtithe,
                ctx.fight,
                &ctx.managers.buff_mgr,
                &mut ctx.managers.ex_point_mgr,
                &gains,
                &ts_event.bloodpool_gain_by_skill_team,
            ) {
                ctx.managers
                    .calculate_mgr
                    .play_step_data(
                        &sync_step,
                        ctx.fight,
                        &mut ctx.mechanics.bloodtithe,
                        &mut ctx.managers.buff_mgr,
                        &mut ctx.managers.ex_point_mgr,
                    )
                    .map_err(anyhow::Error::msg)
                    .ok();
                sync_steps_for_this.push(sync_step);
            }
            sync_steps_per_trigger.push(sync_steps_for_this);
        }

        let mut queue = EventQueue::new();
        for (ts, sync_steps) in trigger_steps.into_iter().zip(sync_steps_per_trigger) {
            queue.push(fight_step_to_event(ts));
            for sync_step in sync_steps {
                queue.push(fight_step_to_event(sync_step));
            }
        }

        let mut buff_mgr = BuffMgr::new();
        let mut ex_point_mgr = ExPointMgr::new();
        let mut event_ctx = EventContext {
            fight: ctx.fight,
            buff_mgr: &mut buff_mgr,
            ex_point_mgr: &mut ex_point_mgr,
        };
        let drained = drain_to_fight_steps(queue.drain(), &mut event_ctx);

        let mut out = vec![root_step.clone()];
        for effect in drained {
            if let Some(step) = effect.fight_step {
                out.push(step);
            }
        }
        out
    }

    pub(crate) fn deleted_buff_ids_from_delta(
        &self,
        before: &[(i64, super::buff_mgr::BuffInstance)],
        after: &[(i64, super::buff_mgr::BuffInstance)],
    ) -> Vec<i32> {
        let mut out = Vec::new();
        let after_keys: HashSet<(i64, i64)> = after.iter().map(|(uid, b)| (*uid, b.uid)).collect();
        for (uid, instance) in before {
            if !after_keys.contains(&(*uid, instance.uid)) {
                if instance.buff_id > 0 && !out.contains(&instance.buff_id) {
                    out.push(instance.buff_id);
                }
                if instance.type_id > 0 && !out.contains(&instance.type_id) {
                    out.push(instance.type_id);
                }
            }
        }
        out
    }

    fn run_passive_phase(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
        config: PassivePhaseConfig,
    ) -> Vec<FightStep> {
        let scope_uids = match &config.scope {
            PhaseScope::Attackers => collected.attacker_uids(),
            PhaseScope::Defenders => collected.defender_uids(),
        };

        let mut steps = match config.skill_set {
            PhaseSkillSet::ExcludeBattleRule | PhaseSkillSet::CombatReactive => {
                let battle_rule_skills = self.collect_battle_rule_skills(ctx.fight);
                let stop_at_first = matches!(config.depth, PhaseDepth::FirstMatch);
                let passive_phase = PhaseFilter::combat();
                let is_defender_sweep = matches!(config.scope, PhaseScope::Defenders);
                let mut out = Vec::new();

                for &uid in &scope_uids {
                    let is_attacker_uid = self.uid_on_attacker_side(ctx.fight, uid);
                    let mut per_entity_effects: Vec<ActEffect> = Vec::new();
                    let mut skill_ids = collected.merged_for(uid);
                    self.extend_with_buff_granted_passives(ctx, uid, &mut skill_ids);
                    if !is_attacker_uid {
                        for sid in &battle_rule_skills {
                            if !skill_ids.contains(sid)
                                && !collected.battle_defender.contains(sid)
                                && !collected.battle_attacker.contains(sid)
                            {
                                skill_ids.push(*sid);
                            }
                        }
                    }
                    for skill_id in skill_ids {
                        if matches!(config.skill_set, PhaseSkillSet::CombatReactive)
                            && !has_combat_reactive_condition(
                                skill_id,
                                CombatPassiveScanMode::RoundSweep,
                            )
                        {
                            continue;
                        }
                        if is_attacker_uid && battle_rule_skills.contains(&skill_id) {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, uid, uid, skill_id, &passive_phase)
                            && !effects.is_empty()
                        {
                            // Defender-side idle sweeps in LIVE do not emit
                            // wrappers for state-machine passives whose only
                            // output is a BuffUpdate marker (e.g. 530000151
                            // cycling between 530000111/530000112 via
                            // NoBuffId gates). LIVE fires these nested inside
                            // actual combat events. Drop marker-only wrappers
                            // from defender sweeps so top-level OURS steps
                            // don't balloon with no-op state ticks.
                            let kept: Vec<ActEffect> = if is_defender_sweep
                                && has_combat_reactive_condition(
                                    skill_id,
                                    CombatPassiveScanMode::RoundSweep,
                                ) {
                                effects
                                    .into_iter()
                                    .filter(|e| !is_marker_only_fight_step_effect(e))
                                    .collect()
                            } else {
                                effects
                            };
                            if kept.is_empty() {
                                continue;
                            }
                            per_entity_effects.extend(kept);
                            if stop_at_first {
                                break;
                            }
                        }
                    }

                    if !per_entity_effects.is_empty() {
                        out.push(build_effect_step(per_entity_effects));
                    }
                }

                out
            }
            PhaseSkillSet::DefenderBootstrap => {
                let defender_uids = collected.defender_uids();
                let mut out = Vec::new();
                let defender_skill_set: std::collections::HashSet<i32> = defender_uids
                    .iter()
                    .flat_map(|uid| collected.merged_for(*uid))
                    .collect();
                let mut ordered_skills: Vec<i32> = Vec::new();

                let mut teammate_alive_skills: Vec<i32> = defender_skill_set
                    .iter()
                    .copied()
                    .filter(|sid| self.is_teammate_alive_self_addbuff(*sid))
                    .collect();
                teammate_alive_skills.sort_unstable();
                ordered_skills.extend(teammate_alive_skills);

                let mut battle_rule_skills: Vec<i32> = self
                    .collect_battle_rule_skills(ctx.fight)
                    .into_iter()
                    .filter(|sid| defender_skill_set.contains(sid))
                    .collect();
                battle_rule_skills.sort_unstable();
                for sid in battle_rule_skills {
                    if !ordered_skills.contains(&sid) {
                        ordered_skills.push(sid);
                    }
                }

                for skill_id in ordered_skills {
                    let mut wrapped = Vec::new();
                    for uid in &defender_uids {
                        let should_try = collected.merged_for(*uid).contains(&skill_id);
                        if !should_try {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &PhaseFilter::combat())
                            && !effects.is_empty()
                        {
                            let inner = build_effect_step(effects);
                            wrapped.push(wrap_step(inner));
                        }
                    }
                    if !wrapped.is_empty() {
                        out.push(build_effect_step(wrapped));
                    }
                }

                out
            }
            PhaseSkillSet::BattleRuleOnly => {
                let attacker_uids: Vec<i64> = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .map(|a| {
                        a.entitys
                            .iter()
                            .chain(a.sub_entitys.iter())
                            .filter(|e| {
                                e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0
                            })
                            .filter_map(|e| e.uid)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut out = Vec::new();
                let attacker_skill_set: std::collections::HashSet<i32> = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .map(|a| {
                        a.entitys
                            .iter()
                            .chain(a.sub_entitys.iter())
                            .filter(|e| {
                                e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0
                            })
                            .flat_map(|e| e.passive_skill.iter().copied())
                            .collect()
                    })
                    .unwrap_or_default();

                let mut battle_rule_skills: Vec<i32> = self
                    .collect_battle_rule_skills(ctx.fight)
                    .into_iter()
                    .filter(|sid| attacker_skill_set.contains(sid))
                    .filter(|sid| {
                        let effect_id = resolve_skill_effect_id(*sid);
                        let cfg = config::configs::get();
                        let cond = cfg
                            .skill_effect
                            .iter()
                            .find(|s| s.id == effect_id)
                            .map(|s| s.condition1.clone())
                            .unwrap_or_default();
                        let (parsed, _) = parse_condition(cond.trim());
                        !matches!(parsed, ConditionType::TargetCareer { .. })
                    })
                    .collect();
                battle_rule_skills.sort_unstable();

                for skill_id in battle_rule_skills {
                    let mut wrapped = Vec::new();
                    for uid in &attacker_uids {
                        let should_try = ctx
                            .fight
                            .attacker
                            .as_ref()
                            .map(|a| {
                                a.entitys
                                    .iter()
                                    .chain(a.sub_entitys.iter())
                                    .find(|e| e.uid == Some(*uid))
                                    .map(|e| e.passive_skill.contains(&skill_id))
                                    .unwrap_or(false)
                            })
                            .unwrap_or(false);
                        if !should_try {
                            continue;
                        }
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &PhaseFilter::combat())
                            && !effects.is_empty()
                        {
                            let inner = build_effect_step(effects);
                            wrapped.push(wrap_step(inner));
                        }
                    }
                    if !wrapped.is_empty() {
                        out.push(build_effect_step(wrapped));
                    }
                }

                out
            }
        };

        if matches!(config.step_shape, PhaseStepShape::FlatIfAllUpdate) {
            steps = split_updates_and_wrap_rest(steps);
        }

        steps
    }

    fn extend_with_buff_granted_passives(
        &self,
        ctx: &FightContext<'_>,
        uid: i64,
        skill_ids: &mut Vec<i32>,
    ) {
        for instance in ctx.managers.buff_mgr.get(uid) {
            crate::state::battle::utils::for_each_buff_feature_chain(
                instance.buff_id,
                |act_type, parts| {
                    let value_start_idx = match act_type {
                        "AddPassiveSkills" => 1,
                        "AddToTarget" | "AddToTargetNoLimit" | "UseDamageSkillAddToTarget" => 2,
                        _ => 0,
                    };
                    if value_start_idx == 0 {
                        return;
                    }
                    for raw in parts.iter().skip(value_start_idx) {
                        for piece in raw.split(',') {
                            if let Ok(skill_id) = piece.trim().parse::<i32>()
                                && skill_id > 0
                            {
                                let resolved_skill_id =
                                    resolve_with_euphoria(ctx.fight, uid, skill_id);
                                if !skill_ids.contains(&resolved_skill_id) {
                                    skill_ids.push(resolved_skill_id);
                                }
                            }
                        }
                    }
                },
            );
        }
    }

    fn uid_on_attacker_side(&self, fight: &Fight, uid: i64) -> bool {
        fight
            .attacker
            .as_ref()
            .map(|a| {
                a.entitys
                    .iter()
                    .chain(a.sub_entitys.iter())
                    .any(|e| e.uid == Some(uid))
            })
            .unwrap_or(false)
    }

    fn collect_battle_rule_skills(&self, fight: &Fight) -> std::collections::HashSet<i32> {
        let mut out = std::collections::HashSet::new();
        let episode_id = fight.episode_id.unwrap_or(0);
        let cfg = config::configs::get();
        let Some(battle_id) = cfg
            .episode
            .iter()
            .find(|e| e.id == episode_id)
            .map(|e| e.battle_id)
        else {
            return out;
        };
        let Some(battle) = cfg.battle.iter().find(|b| b.id == battle_id) else {
            return out;
        };
        if battle.addition_rule.is_empty() {
            return out;
        }

        for entry in battle.addition_rule.split('|') {
            let mut parts = entry.split('#');
            let Some(prefix) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            let Some(id) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            if !(1..=3).contains(&prefix) {
                continue;
            }
            let Some(rule) = cfg.rule.iter().find(|r| r.id == id) else {
                continue;
            };
            let sid = rule.effect.parse::<i32>().ok().unwrap_or(0);
            if sid != 0 {
                out.insert(sid);
            }
        }

        out
    }

    pub(crate) fn first_alive_defender_uid(&self, fight: &Fight) -> Option<i64> {
        fight
            .defender
            .as_ref()
            .and_then(|d| {
                d.entitys
                    .iter()
                    .chain(d.sub_entitys.iter())
                    .find(|e| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0)
            })
            .and_then(|e| e.uid)
    }

    fn is_teammate_alive_self_addbuff(&self, skill_id: i32) -> bool {
        let effect_id = resolve_skill_effect_id(skill_id);
        let cfg = config::configs::get();
        let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
            return false;
        };
        let (cond, _) = parse_condition(skill.condition1.trim());
        matches!(cond, ConditionType::TeammateAlive { .. })
            && skill.behavior_target1.trim() == "103"
            && skill.behavior1.trim().starts_with("1#")
    }
}

