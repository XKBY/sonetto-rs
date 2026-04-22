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
    context::{FightContext, RoundContext},
    fight_step::{FightStepBuilder, split_step_by_effect_limit, wrap_step},
    manager::{
        buff_mgr::{
            DEFENDER_BUFF_UID_START, reset_buff_uid_to, sync_buff_uid_counters_from_fight,
            sync_from_fight_preserve_runtime as sync_buffs_from_fight,
        },
        card_mgr::FightCardMgr,
        ex_point_mgr::{build_ex_point_info, sync_from_fight, sync_to_fight},
        traits::Manager,
    },
    mechanics::{bloodtithe, channel as channel_mechanics, injury_counter, magic_circle},
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
        PhaseFilter,
        cache::resolve_skill_effect_id,
        classification::{CombatPassiveScanMode, has_combat_reactive_condition},
        condition::parser::parse_condition,
    },
    steps::{broadcast, ex_gain, step_normalize, trigger_embed},
    trigger::{
        combat::{event_from_step, fire_combat_triggers},
        passes::{build_belief_gain_step, sync_blood_value_baseline},
    },
    types::effects::EffectType,
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

#[derive(Default, Debug, Clone)]
pub struct FightRoundMgr;

impl FightRoundMgr {
    pub fn new() -> Self {
        Self
    }

    fn step_contains_magic_circle_add(&self, step: &FightStep) -> bool {
        step.act_effect.iter().any(|effect| {
            effect.effect_type
                == Some(crate::state::battle::types::effects::EffectType::MagicCircleAdd as i32)
                || effect
                    .fight_step
                    .as_ref()
                    .map(|child| self.step_contains_magic_circle_add(child))
                    .unwrap_or(false)
        })
    }

    fn inline_magic_circle_root_wrapper(&self, host_step: &mut FightStep) -> bool {
        let Some(idx) = host_step.act_effect.iter().position(|effect| {
            effect.effect_type == Some(162)
                && effect
                    .fight_step
                    .as_ref()
                    .map(|step| {
                        step.act_type == Some(fight_step::ActType::Skill as i32)
                            && self.step_contains_magic_circle_add(step)
                    })
                    .unwrap_or(false)
        }) else {
            return false;
        };

        let Some(inner) = host_step
            .act_effect
            .remove(idx)
            .fight_step
            .filter(|step| step.act_type == Some(fight_step::ActType::Skill as i32))
        else {
            return false;
        };

        host_step.act_effect.splice(idx..idx, inner.act_effect);
        true
    }

    fn host_trigger_insert_index(&self, host_step: &FightStep) -> usize {
        host_step
            .act_effect
            .iter()
            .position(|effect| {
                effect.effect_type
                    == Some(crate::state::battle::types::effects::EffectType::MagicCircleAdd as i32)
            })
            .map(|idx| idx + 1)
            .unwrap_or_else(|| trigger_embed::find_trigger_insert_index(&host_step.act_effect))
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
        sync_buff_uid_counters_from_fight(ctx.fight);

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
        let attacker_uid_checkpoint = self.max_side_buff_uid(ctx.fight, false);
        let mut defender_uid_checkpoint = self.max_side_buff_uid(ctx.fight, true);
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
        let next_round_begin_step = vec![
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
        ];
        open.steps = open
            .steps
            .into_iter()
            .flat_map(split_step_by_effect_limit)
            .collect();

        Ok(FightRound {
            fight_step: open.steps,
            act_point: Some(3),
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
        for oper in operations {
            let ex_step_after_op = ex_gain::pre_operation_ex_gain(ctx, state, &oper);
            let buff_snapshot_before = ctx.managers.buff_mgr.all_instances();
            let step = card_mgr.execute_operation(rng, ctx, state, oper).await?;
            if step.act_type.unwrap_or(0) == 0 {
                continue;
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
            self.inline_magic_circle_root_wrapper(&mut host_step);
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
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
                    let mut top_level_prefix: Vec<ActEffect> = Vec::new();
                    let mut nested_embedded: Vec<ActEffect> = Vec::new();
                    for trigger_step in expanded_steps.into_iter().skip(1) {
                        let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
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
                    let insert_at = self.host_trigger_insert_index(&host_step);
                    host_step
                        .act_effect
                        .splice(insert_at..insert_at, embedded_steps);
                }
            }
            let monitor_embeds =
                channel_mechanics::build_monitor_continue_channel_embeds(ctx, &step, &host_step);
            if !monitor_embeds.is_empty() {
                let insert_at = self.host_trigger_insert_index(&host_step);
                host_step
                    .act_effect
                    .splice(insert_at..insert_at, monitor_embeds);
            }
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
            trigger_embed::flatten_self_nested_skill_effects(&mut host_step);
            trigger_embed::normalize_player_skill_effect_order(&mut host_step);
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
            let expanded_steps =
                self.expand_trigger_chain(ctx, collected, &step, &runtime_deleted_buff_ids);
            let is_embedded_skill_host = step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.from_id.unwrap_or(0) >= 0;
            if !is_embedded_skill_host {
                steps.extend(expanded_steps);
                continue;
            }

            let mut host_step = step.clone();
            self.inline_magic_circle_root_wrapper(&mut host_step);
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
                    let mut top_level_prefix: Vec<ActEffect> = Vec::new();
                    let mut nested_embedded: Vec<ActEffect> = Vec::new();
                    for trigger_step in expanded_steps.into_iter().skip(1) {
                        let embedded = trigger_embed::trigger_step_to_embedded_effect(trigger_step);
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
                    let insert_at = self.host_trigger_insert_index(&host_step);
                    host_step
                        .act_effect
                        .splice(insert_at..insert_at, embedded_steps);
                }
            }
            magic_circle::apply_magic_circle_self_skill_embeds(ctx, &mut host_step);
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
            for step in bloodtithe::build_round_transition_bloodtithe_steps(self, ctx, collected) {
                self.apply_step_and_maybe_sync(ctx, &step, true)?;
                steps.push(step);
            }
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

        if self.check_battle_end(ctx.fight) {
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
            // Preview one duration tick for attacker-side round-end broadcast only.
            let mut broadcast = if ctx.fight.cur_round.unwrap_or(1) == 1 {
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
        let mut out = vec![root_step.clone()];
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
        for ts in trigger_steps {
            ctx.managers
                .calculate_mgr
                .play_step_data(
                    &ts,
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
            out.push(ts);
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
                        out.push(sync_step);
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
                out.push(sync_step);
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

    fn max_side_buff_uid(&self, fight: &Fight, defender: bool) -> i64 {
        let side = if defender {
            fight.defender.as_ref()
        } else {
            fight.attacker.as_ref()
        };
        side.map(|team| {
            team.entitys
                .iter()
                .chain(team.sub_entitys.iter())
                .flat_map(|e| e.buffs.iter())
                .filter_map(|b| b.uid)
                .max()
                .unwrap_or(if defender { DEFENDER_BUFF_UID_START } else { 0 })
        })
        .unwrap_or(if defender { DEFENDER_BUFF_UID_START } else { 0 })
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
                            per_entity_effects.extend(effects);
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
                                && !skill_ids.contains(&skill_id)
                            {
                                skill_ids.push(skill_id);
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
