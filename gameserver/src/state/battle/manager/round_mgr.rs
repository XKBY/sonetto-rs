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
    card::CardOpType,
    context::{FightContext, RoundContext},
    fight_step::{
        FightStepBuilder, effect_container_step, make_skill_step, split_step_by_effect_limit,
        wrap_step,
    },
    manager::{
        buff_mgr::next_buff_uid_for_target,
        card_mgr::FightCardMgr,
        ex_point_mgr::{build_ex_point_info, sync_to_fight},
    },
    mechanics::{self, injury_counter},
    passives::{
        collector::CollectedPassives, steps::skill::execute_skill as execute_passive_skill,
    },
    phase,
    round::{
        PassivePhaseConfig, PhaseDepth, PhaseScope, PhaseSkillSet, PhaseStepShape, RoundState,
        step_shape::{build_effect_step, split_updates_and_wrap_rest},
    },
    round_end_emission,
    skill::{
        cache::resolve_skill_effect_id,
        classification::{CombatPassiveScanMode, has_combat_reactive_condition},
        condition::{misc::HriEvalGuard, parser::parse_condition},
        euphoria::resolve_with_euphoria,
    },
    step_walker,
    trigger::{
        combat::expand_trigger_chain_from_root_step,
    },
    types::effects::EffectType,
};

pub(crate) enum BattleEndState {
    Ongoing,
    WaveCleared, // all enemies dead, more waves remain
    Victory,     // all waves cleared
    Defeat,      // all heroes dead
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

pub(crate) fn skill_has_no_act_round_condition(skill_id: i32) -> bool {
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

pub(crate) fn active_cloth_level(fight: &Fight) -> Option<config::cloth_level::ClothLevel> {
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

pub(crate) fn parse_cloth_recover_delta(recover: &str, round_index: i32) -> i32 {
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

pub(crate) fn seed_attacker_power_from_cloth(
    fight: &mut Fight,
    cloth: &config::cloth_level::ClothLevel,
) {
    if let Some(attacker) = fight.attacker.as_mut()
        && attacker.power.is_none()
    {
        attacker.power = Some(cloth.initial.max(0));
    }
}

pub(crate) fn apply_cloth_power_delta(
    fight: &mut Fight,
    cloth: &config::cloth_level::ClothLevel,
    delta: i32,
) {
    let Some(attacker) = fight.attacker.as_mut() else {
        return;
    };
    let current = attacker.power.unwrap_or(cloth.initial.max(0));
    let next = (current + delta).clamp(0, cloth.max_power.max(0));
    attacker.power = Some(next);
}

pub(crate) fn cloth_power_delta_for_operation(
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

    pub(crate) fn collect_round_tied_defender_passive_steps(
        &self,
        ctx: &mut FightContext<'_>,
        collected: &CollectedPassives,
    ) -> Vec<ActEffect> {
        let cur_round = crate::state::battle::round_state::simulated_round();
        let passive_phase = ctx.combat_phase();
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

    /// When a player-side SKILL emission deals damage to enemies AND
    /// the enemy side has a `BeAttacked`-type battle-rule passive
    /// registered as an `ai_card` (currently `530000411` in the
    /// fixtures we cover), graft a reactive wrapper for each damaged
    /// enemy onto the host's `act_effect`. The duplicate guard on
    /// the existing `530000411` child prevents this from over-firing
    /// when the standard trigger pipeline already embedded the
    /// reactive — so removing the previous Recoleta-ult act_id gate
    /// is safe: any player ult whose damage chain reaches a
    /// `BeAttacked`-passive boss gets the same shape without code
    /// changes.
    ///
    /// TODO(event-queue): EventQueue Phase 4
    /// (`SkillEmitKind::EventTriggered`) eventually replaces this
    /// post-emission graft with a damage-event-tied reactive emission
    /// during drain. Until that lands, this is the cleanest non-
    /// hardcoded approximation of the LIVE shape.
    pub(crate) fn graft_be_attacked_reactives_onto_player_host(
        &self,
        state: &RoundState,
        host_step: &mut FightStep,
    ) {
        const BE_ATTACKED_REACTIVE_ACT_ID: i32 = 530000411;

        if host_step.act_type != Some(fight_step::ActType::Skill as i32)
            || host_step.from_id.unwrap_or(0) <= 0
        {
            return;
        }
        // Only graft on ultimate-skill bodies. LIVE never nests
        // `BeAttacked` reactives inside basic-skill hosts — they fire
        // once per round at top-level via the natural passive pipeline.
        // Config-driven: skill_effect.isBigSkill == 1 marks the
        // ultimate body (e.g. 31140131 for Recoleta) vs basics
        // (31140111 / 31140121, isBigSkill == 0).
        let host_act_id = host_step.act_id.unwrap_or(0);
        let host_effect_id = resolve_skill_effect_id(host_act_id);
        let host_is_big_skill = config::configs::get()
            .skill_effect
            .get(host_effect_id)
            .map(|cfg| cfg.is_big_skill == 1)
            .unwrap_or(false);
        if !host_is_big_skill {
            return;
        }
        if host_step.act_effect.iter().any(|effect| {
            step_walker::wrapped_skill_from_effect(effect)
                .map(|step| step.act_id == Some(BE_ATTACKED_REACTIVE_ACT_ID))
                .unwrap_or(false)
        }) {
            return;
        }

        let reactive_caster_uid = state
            .ai_cards
            .iter()
            .find(|card| {
                card.skill_id == Some(BE_ATTACKED_REACTIVE_ACT_ID) && card.uid.unwrap_or(0) < 0
            })
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
                    BE_ATTACKED_REACTIVE_ACT_ID,
                    buff_uid,
                    0,
                    0,
                );
                wrap_step(effect_container_step(
                    reactive_caster_uid,
                    target_uid,
                    BE_ATTACKED_REACTIVE_ACT_ID,
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

    #[allow(clippy::too_many_arguments)]
    pub async fn process_round_with_replay(
        &self,
        rng: &mut StdRng,
        round_ctx: &mut RoundContext<'_, '_>,
        card_mgr: &mut FightCardMgr,
        operations: Vec<BeginRoundOper>,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
        ai_override_steps: Option<Vec<FightStep>>,
        replay_selected_cards: Option<Vec<CardInfo>>,
        replay_silent_ops: Option<Vec<bool>>,
    ) -> Result<FightRound> {
        let mut open = phase::round_open::run(
            round_ctx,
            &current_deck,
            &ai_deck,
            ai_override_steps.as_deref(),
            &operations,
            replay_selected_cards.as_deref(),
            replay_silent_ops.as_deref(),
        );
        let ctx = &mut *round_ctx.fight_ctx;

        phase::player_actions::run(
            self,
            rng,
            ctx,
            card_mgr,
            &mut open.state,
            operations,
            &open.collected,
            &mut open.steps,
        )
        .await?;

        phase::non_terminal_round::run(
            self,
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
        round_end_emission::merge_post_turn_reactives_into_host(&mut open.steps);
        mechanics::nautika::strip_duplicate_change_round_markers(&mut open.steps);
        mechanics::nautika::consolidate_into_bundle(ctx.fight, &mut open.steps);
        mechanics::nautika::strip_redundant_post_round_emissions(ctx.fight, &mut open.steps);

        if crate::state::battle::emission_timeline::EmissionTimeline::dump_enabled() {
            eprint!("{}", ctx.mechanics.emission_timeline.dump());
        }

        self.build_round_output(round_ctx, open, current_deck, ai_deck)
    }

    fn build_round_output(
        &self,
        round_ctx: &mut RoundContext<'_, '_>,
        mut open: phase::round_open::RoundOpenPhaseData,
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
    pub(crate) fn apply_passive_phase(
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

    pub(crate) fn check_battle_state(
        &self,
        fight: &Fight,
        cur_wave: i32,
        max_wave: i32,
    ) -> BattleEndState {
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

    pub(crate) fn check_battle_end(&self, fight: &Fight) -> bool {
        let cur_wave = fight.cur_wave.unwrap_or(1);
        let max_wave = self.get_max_wave(fight);
        matches!(
            self.check_battle_state(fight, cur_wave, max_wave),
            BattleEndState::Victory | BattleEndState::Defeat
        )
    }

    pub(crate) fn get_max_wave(&self, fight: &Fight) -> i32 {
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
        expand_trigger_chain_from_root_step(ctx, collected, root_step, runtime_deleted_buff_ids)
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
                let passive_phase = ctx.combat_phase();
                let is_defender_sweep = matches!(config.scope, PhaseScope::Defenders);
                let mut out = Vec::new();

                for &uid in &scope_uids {
                    let is_attacker_uid = self.uid_on_attacker_side(ctx.fight, uid);
                    let mut per_entity_effects: Vec<ActEffect> = Vec::new();
                    let mut skill_ids = collected.merged_for(uid);
                    self.extend_with_buff_granted_passives(ctx, uid, &mut skill_ids);
                    self.extend_with_magic_circle_enemy_skills(ctx, uid, &mut skill_ids);
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
                            && skill_id != 30630171
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
                        let timeline_phase = match config.skill_set {
                            PhaseSkillSet::CombatReactive => {
                                crate::state::battle::emission_timeline::EmissionPhase::CombatReactive
                            }
                            _ => crate::state::battle::emission_timeline::EmissionPhase::RoundPassiveSweep,
                        };
                        ctx.mechanics.emission_timeline.record(
                            timeline_phase,
                            uid,
                            skill_id,
                            0,
                            None,
                            None,
                        );
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
                        ctx.mechanics.emission_timeline.record(
                            crate::state::battle::emission_timeline::EmissionPhase::DefenderBootstrap,
                            *uid,
                            skill_id,
                            0,
                            None,
                            None,
                        );
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &ctx.combat_phase())
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

                // Skills with addition_rule prefix=3 are defender-side rules
                // (e.g. boss state cycle 530000151). Even though attackers
                // carry them in passive_skill (LIVE-supplied), LIVE engine
                // does not fire them from attacker entities. Skip them here
                // so the attacker BattleRuleOnly pass doesn't surface
                // standalone self-cast wrappers LIVE never emits.
                let defender_only_rules: std::collections::HashSet<i32> =
                    crate::state::battle::passives::collector::collect_battle_passives(
                        ctx.fight.battle_id.unwrap_or(0),
                    )
                    .defender
                    .into_iter()
                    .collect();
                let mut battle_rule_skills: Vec<i32> = self
                    .collect_battle_rule_skills(ctx.fight)
                    .into_iter()
                    .filter(|sid| attacker_skill_set.contains(sid))
                    .filter(|sid| !defender_only_rules.contains(sid))
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
                        ctx.mechanics.emission_timeline.record(
                            crate::state::battle::emission_timeline::EmissionPhase::BattleRuleOnly,
                            *uid,
                            skill_id,
                            0,
                            None,
                            None,
                        );
                        if let Ok(effects) =
                            execute_passive_skill(ctx, *uid, *uid, skill_id, &ctx.combat_phase())
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

    /// Magic-circle `enemy_skills` round-start delivery. When a circle is
    /// active and `uid` sits on the opposite side from the circle owner,
    /// every id in the circle config's `enemy_skills` field becomes a
    /// passive on `uid` for this sweep.
    ///
    /// Tuesday's `magic_circle 22100003` is the fixture caller — its
    /// `enemy_skills="30980151"` advertises the round-start Poison
    /// settle to each enemy of the circle owner. LIVE always emits
    /// `30980151` with `fromId == toId == defender_uid` (`-5` in r5,
    /// `-7` in r6/r8/r9), confirming the carrier is each enemy, not
    /// the array owner. The skill itself has `cond=101 beh=60073#1`,
    /// so the round-start scope gate (`scope::is_round_start_only`)
    /// already blocks any combat-event path from re-firing it.
    fn extend_with_magic_circle_enemy_skills(
        &self,
        ctx: &FightContext<'_>,
        uid: i64,
        skill_ids: &mut Vec<i32>,
    ) {
        let Some(circle) = ctx.fight.magic_circle.as_ref() else {
            return;
        };
        let Some(circle_id) = circle.magic_circle_id else {
            return;
        };
        if circle.round.unwrap_or(0) == 0 {
            return;
        }
        let create_uid = circle.create_uid.unwrap_or(0);
        if create_uid == 0 || uid == 0 || create_uid.signum() == uid.signum() {
            return;
        }
        let cfg = config::configs::get();
        let Some(circle_cfg) = cfg.magic_circle.get(circle_id) else {
            return;
        };
        for piece in circle_cfg
            .enemy_skills
            .split(['|', ',', ';', '#'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let Ok(skill_id) = piece.parse::<i32>() else {
                continue;
            };
            if skill_id <= 0 {
                continue;
            }
            let resolved = resolve_with_euphoria(ctx.fight, uid, skill_id);
            if !skill_ids.contains(&resolved) {
                skill_ids.push(resolved);
            }
        }
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
                    if act_type != "AddPassiveSkills" {
                        return;
                    }
                    for raw in parts.iter().skip(1) {
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

    pub(crate) fn collect_battle_rule_skills(
        &self,
        fight: &Fight,
    ) -> std::collections::HashSet<i32> {
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
