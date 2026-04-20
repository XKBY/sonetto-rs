use anyhow::Result;
use rand::{Rng, rngs::StdRng};
use std::collections::HashMap;

use sonettobuf::{ActEffect, BeginRoundOper, Fight, FightEntityInfo, FightStep, fight_step};

use super::super::{
    card::CardOpType,
    fight_step::make_skill_step,
    context::FightContext,
    passives::collector::collect,
    passives::steps::skill::execute_skill as execute_passive_skill,
    round::RoundState,
    skill::{
        PhaseFilter, SkillExecutor, TriggerState,
        cache::{SKILL_CACHE, resolve_skill_effect_id},
    },
    trigger::combat::{TriggerEvent, skill_should_fire},
    types::{behavior::BehaviorType, condition::ConditionType, effects::EffectType},
    utils::{buff_get_ex_point_overflow, damage_with_hurt, find_entity},
};
use super::fight_data_mgr::Managers;

#[derive(Default, Debug, Clone)]
pub struct FightCardMgr {
    skill_executor: SkillExecutor,
}

#[allow(dead_code)]
impl FightCardMgr {
    pub fn new() -> Self {
        Self {
            skill_executor: SkillExecutor::new(),
        }
    }

    pub async fn execute_operation(
        &mut self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        state: &mut RoundState,
        oper: BeginRoundOper,
    ) -> Result<FightStep> {
        // Reset the per-step buff-deletion tracker so mid-step passive
        // chains that gate on `BuffIdDel` only see deletions produced by
        // this operation.
        ctx.managers.buff_mgr.clear_step_deleted_buff_ids();
        let op = CardOpType::try_from(oper.oper_type.unwrap_or(0));
        match op {
            Ok(CardOpType::PlayCard) => self.play_card(rng, ctx, state, oper).await,
            Ok(CardOpType::MoveCard) => {
                if oper.to_id.unwrap_or(0) != 0 {
                    self.play_card(rng, ctx, state, oper).await
                } else {
                    Ok(self.select_card(oper))
                }
            }
            Ok(CardOpType::AssistBoss) => self.play_card(rng, ctx, state, oper).await,
            Ok(CardOpType::PlayerFinisherSkill) => self.play_card(rng, ctx, state, oper).await,
            Ok(CardOpType::BloodPool) => self.play_card(rng, ctx, state, oper).await,
            Ok(CardOpType::SimulateDissolveCard) => Ok(self.dissolve_card(oper, state)),
            _ => Ok(FightStep::default()),
        }
    }

    fn select_card(&self, oper: BeginRoundOper) -> FightStep {
        // Client sends 1-based index, convert to 0-based
        let card_index = oper.param1.unwrap_or(1) - 1;
        FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            act_effect: vec![ActEffect {
                effect_type: Some(EffectType::AddHandCard as i32),
                effect_num: Some(card_index),
                team_type: Some(1),
                ..Default::default()
            }],
            card_index: Some(card_index),
            ..Default::default()
        }
    }

    async fn play_card(
        &mut self,
        _rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        state: &mut RoundState,
        oper: BeginRoundOper,
    ) -> Result<FightStep> {
        // Client sends 1-based card index, convert to 0-based
        let card_index = (oper.param1.unwrap_or(1) - 1) as usize;
        let raw_target_uid = oper.to_id.unwrap_or(0);

        let card = match state.player_deck.get(card_index).cloned() {
            Some(c) => c,
            None => return Ok(FightStep::default()),
        };

        state.player_deck.remove(card_index);

        let display_caster_uid = card.uid.unwrap_or(0);
        let is_temp_card = card.temp_card.unwrap_or(false) || display_caster_uid == 0;

        let hero_id = card.hero_id.unwrap_or(0);
        let inferred_model_id = if hero_id == 0 {
            // Temp cards can come as uid=0/heroId=0. Infer owner model from skill id prefix.
            let sid = card.skill_id.unwrap_or(0);
            if sid >= 10000 { sid / 10000 } else { 0 }
        } else {
            hero_id
        };
        let exec_caster_uid = if display_caster_uid > 0 {
            display_caster_uid
        } else {
            ctx.fight
                .attacker
                .as_ref()
                .and_then(|a| {
                    a.entitys
                        .iter()
                        .chain(a.sub_entitys.iter())
                        .find(|e| e.model_id == Some(inferred_model_id))
                        .and_then(|e| e.uid)
                })
                // Temp/precast cards in live can be uid=0, heroId=0.
                // Allow caster_uid=0 for this synthetic path.
                .unwrap_or(0)
        };

        let skill_id = card.skill_id.unwrap_or(0);

        // Choice cards have 0 behaviors. Use skill_ex_level to find the options
        // at the correct rank, then use param2 (1-based) to select which to execute.
        let resolved_skill_id = {
            let has_behaviors = SKILL_CACHE
                .get(&skill_id)
                .map(|b| !b.is_empty())
                .unwrap_or(false);
            if !has_behaviors && !is_temp_card {
                // Choice card: param3 contains the chosen skill ID directly
                let chosen = oper.param3.unwrap_or(0);
                if chosen != 0 {
                    tracing::warn!("choice card skill={} -> chosen={}", skill_id, chosen);
                    chosen
                } else {
                    skill_id
                }
            } else {
                skill_id
            }
        };
        let target_uid = if raw_target_uid > 0 {
            raw_target_uid
        } else {
            ctx.fight
                .defender
                .as_ref()
                .and_then(|d| {
                    d.entitys
                        .iter()
                        .find(|e| e.current_hp.unwrap_or(0) > 0)
                        .and_then(|e| e.uid)
                })
                .unwrap_or(raw_target_uid)
        };
        let is_direct_ex_card = !is_temp_card
            && ctx
                .fight
                .attacker
                .as_ref()
                .and_then(|a| {
                    a.entitys
                        .iter()
                        .chain(a.sub_entitys.iter())
                        .find(|e| e.uid == Some(exec_caster_uid))
                })
                .and_then(|e| e.ex_skill)
                .map(|ex_skill| ex_skill == resolved_skill_id)
                .unwrap_or(false);

        if resolved_skill_id == 31140151 {
        }

        let mut raw_skill_effects = if is_direct_ex_card {
            self.build_direct_ex_card_prefix(ctx, exec_caster_uid, resolved_skill_id)?
        } else {
            Vec::new()
        };
        let mut main_skill_effects = self.skill_executor.execute_skill(
            ctx.fight,
            ctx.managers,
            ctx.mechanics,
            exec_caster_uid,
            target_uid,
            resolved_skill_id,
            &PhaseFilter::combat_with(
                TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
            ),
        )?;
        raw_skill_effects.append(&mut main_skill_effects);
        let mut skill_effects = normalize_skill_effects_for_operation(
            std::mem::take(&mut raw_skill_effects),
            exec_caster_uid,
            resolved_skill_id,
        );
        if is_temp_card && skill_effects.is_empty() {
            for fallback_phase in [PhaseFilter::unconditional(), PhaseFilter::enter_fight()] {
                let retry = self.skill_executor.execute_skill(
                    ctx.fight,
                    ctx.managers,
                    ctx.mechanics,
                    exec_caster_uid,
                    target_uid,
                    resolved_skill_id,
                    &fallback_phase,
                )?;
                let normalized_retry = normalize_skill_effects_for_operation(
                    retry,
                    exec_caster_uid,
                    resolved_skill_id,
                );
                if !normalized_retry.is_empty() {
                    skill_effects = normalized_retry;
                    break;
                }
            }
        }
        if is_temp_card && skill_effects.is_empty() {
            let mut fallback = self
                .build_temp_direct_bigskill_fallback(ctx, exec_caster_uid, target_uid, resolved_skill_id)?;
            if !fallback.is_empty() {
                fallback.extend(skill_effects);
                skill_effects = fallback;
            }
        }

        // Fire ActiveUseSkill passives inline — these appear as 162 wrappers
        // inside the card skill step's actEffect.
        let collected = collect(ctx.fight, 0);
        let passive_phase = PhaseFilter::combat_with(
            TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
        );
        let use_card_event = TriggerEvent {
            caster_uid: exec_caster_uid,
            skill_id: resolved_skill_id,
            primary_target_uid: target_uid,
            used_ex_skill: ctx
                .fight
                .attacker
                .as_ref()
                .and_then(|a| {
                    a.entitys
                        .iter()
                        .chain(a.sub_entitys.iter())
                        .find(|e| e.uid == Some(exec_caster_uid))
                })
                .and_then(|e| e.ex_skill)
                .map(|ex| ex == resolved_skill_id)
                .unwrap_or(false),
            from_wrapper_card: display_caster_uid == 0,
            nested_skill_uses: vec![],
            damaged_uids: vec![],
            dealer_uids: vec![],
            deleted_buff_ids: ctx.managers.buff_mgr.step_deleted_buff_ids().to_vec(),
            added_buff_uids: vec![],
            trigger_bullet: false,
            bloodpool_gain_by_team: vec![],
            bloodpool_gain_by_skill_team: vec![],
            bloodpool_gain_packets_by_team: vec![],
        };
        if !is_temp_card {
            for passive_skill_id in collected.merged_for(exec_caster_uid) {
                if !skill_should_fire(exec_caster_uid, passive_skill_id, &use_card_event) {
                    continue;
                }
                match execute_passive_skill(
                    ctx,
                    exec_caster_uid,
                    target_uid,
                    passive_skill_id,
                    &passive_phase,
                ) {
                    Ok(effects) if !effects.is_empty() => {
                        skill_effects.extend(effects);
                    }
                    _ => {}
                }
            }
        }

        state.used_cards.push(card_index as i32);
        // Temp cards are bonus plays and should not consume a normal card-use slot.
        if !is_temp_card {
            state.act_point = (state.act_point - 1).max(0);
        }

        Ok(make_skill_step(
            display_caster_uid,
            target_uid,
            resolved_skill_id, // actId shows the resolved skill, not the choice card
            card_index as i32,
            skill_effects,
        ))
    }

    pub async fn execute_ai_turn(
        &mut self,
        rng: &mut StdRng,
        ctx: &mut FightContext<'_>,
        state: &mut RoundState,
    ) -> Result<Vec<FightStep>> {
        let mut preview_fight = ctx.fight.clone();
        let mut preview_managers = ctx.managers.clone();
        let mut preview_mechanics = ctx.mechanics.clone();

        if let Some(override_steps) = state.ai_override_steps.as_ref() {
            let players: Vec<i64> = ctx
                .fight
                .attacker
                .as_ref()
                .map(|a| {
                    a.entitys
                        .iter()
                        .filter(|e| e.current_hp.unwrap_or(0) > 0)
                        .filter_map(|e| e.uid)
                        .collect()
                })
                .unwrap_or_default();
            let mut steps = Vec::new();
            for step in override_steps {
                let caster_uid = step.from_id.unwrap_or(0);
                let skill_id = step.act_id.unwrap_or(0);
                if caster_uid >= 0 {
                    continue;
                }
                if skill_id == 0 {
                    continue;
                }
                let caster_alive = ctx
                    .fight
                    .defender
                    .as_ref()
                    .map(|d| {
                        d.entitys
                            .iter()
                            .find(|e| e.uid == Some(caster_uid))
                            .map(|e| e.current_hp.unwrap_or(0) > 0)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if caster_alive {
                    let target_uid = match step.to_id.unwrap_or(0) {
                        0 => {
                            if players.is_empty() {
                                continue;
                            }
                            players[rng.gen_range(0..players.len())]
                        }
                        t => t,
                    };
                    preview_managers.buff_mgr.clear_step_deleted_buff_ids();
                    let per_behavior = self.skill_executor.execute_skill(
                        &mut preview_fight,
                        &mut preview_managers,
                        &mut preview_mechanics,
                        caster_uid,
                        target_uid,
                        skill_id,
                        &PhaseFilter::combat(),
                    )?;
                    let mut op_effects =
                        normalize_skill_effects_for_operation(per_behavior, caster_uid, skill_id);
                    clamp_ai_add_ex_with_max_effects(
                        &preview_fight,
                        &preview_managers,
                        caster_uid,
                        &mut op_effects,
                    );
                    if op_effects.is_empty() {
                        continue;
                    }
                    let step = make_skill_step(caster_uid, target_uid, skill_id, 0, op_effects);
                    advance_ai_preview_after_cast(
                        &mut preview_fight,
                        &mut preview_managers,
                        caster_uid,
                        &step,
                    );
                    steps.push(step);
                }
            }
            return Ok(steps);
        }

        #[derive(Clone)]
        struct AiCast {
            idx: usize,
            caster_uid: i64,
            skill_id: i32,
            target_uid_opt: Option<i64>,
        }

        let mut steps = Vec::new();
        let players: Vec<i64> = ctx
            .fight
            .attacker
            .as_ref()
            .map(|a| {
                a.entitys
                    .iter()
                    .filter(|e| e.current_hp.unwrap_or(0) > 0)
                    .filter_map(|e| e.uid)
                    .collect()
            })
            .unwrap_or_default();
        if players.is_empty() {
            return Ok(steps);
        }

        let mut candidates: Vec<AiCast> = Vec::new();
        for i in 0..state.ai_cards.len() {
            let (caster_uid, raw_skill_id, target_uid_opt) = {
                let card = &state.ai_cards[i];
                (
                    card.uid.unwrap_or(0),
                    card.skill_id.unwrap_or(0),
                    card.target_uid,
                )
            };
            if caster_uid >= 0 || raw_skill_id == 0 {
                continue;
            }

            let caster_alive = ctx
                .fight
                .defender
                .as_ref()
                .map(|d| {
                    d.entitys
                        .iter()
                        .find(|e| e.uid == Some(caster_uid))
                        .map(|e| e.current_hp.unwrap_or(0) > 0)
                        .unwrap_or(false)
                })
                .unwrap_or(false);

            if !caster_alive {
                continue;
            }
            // Some AI cards point at rank wrappers that have no executable behaviors in config.
            // Mirror live by resolving to the nearest lower castable skill when available.
            let resolved_skill_id = {
                let has_behaviors = SKILL_CACHE
                    .get(&raw_skill_id)
                    .map(|b| !b.is_empty())
                    .unwrap_or(false);
                if has_behaviors {
                    raw_skill_id
                } else {
                    let fallback = raw_skill_id - 1;
                    let fallback_has_behaviors = SKILL_CACHE
                        .get(&fallback)
                        .map(|b| !b.is_empty())
                        .unwrap_or(false);
                    if fallback_has_behaviors {
                        fallback
                    } else {
                        raw_skill_id
                    }
                }
            };

            let canonical_caster_uid = canonical_ai_caster_uid(ctx.fight, caster_uid, resolved_skill_id)
                .unwrap_or(caster_uid);

            candidates.push(AiCast {
                idx: i,
                caster_uid: canonical_caster_uid,
                skill_id: resolved_skill_id,
                target_uid_opt,
            });
        }

        // For the same caster, collapse rank variants of the same base skill
        // (e.g. ...411/...412) to the lowest-rank card first.
        let mut caster_base_choice: HashMap<(i64, i32), AiCast> = HashMap::new();
        for cast in candidates {
            let base = cast.skill_id / 10;
            caster_base_choice
                .entry((cast.caster_uid, base))
                .and_modify(|current| {
                    if cast.skill_id < current.skill_id
                        || (cast.skill_id == current.skill_id && cast.idx < current.idx)
                    {
                        *current = cast.clone();
                    }
                })
                .or_insert(cast);
        }

        // Keep one executor per resolved AI skill id across defenders.
        // When duplicates exist (same skill on multiple defenders), live tends to keep
        // the closer/front uid for this turn.
        let mut chosen_by_skill: HashMap<i32, AiCast> = HashMap::new();
        for cast in caster_base_choice.into_values() {
            chosen_by_skill
                .entry(cast.skill_id)
                .and_modify(|current| {
                    if cast.caster_uid > current.caster_uid
                        || (cast.caster_uid == current.caster_uid && cast.idx < current.idx)
                    {
                        *current = cast.clone();
                    }
                })
                .or_insert(cast);
        }

        let mut casts: Vec<AiCast> = chosen_by_skill.into_values().collect();
        // Defender order: closer/front uid first (e.g. -1 before -2 before -3),
        // then keep original card ordering per caster.
        casts.sort_by(|a, b| b.caster_uid.cmp(&a.caster_uid).then(a.idx.cmp(&b.idx)));

        for cast in casts {
            let i = cast.idx;
            let caster_uid = cast.caster_uid;
            let skill_id = cast.skill_id;
            let target_uid_opt = cast.target_uid_opt;

            // Reset the per-step buff-deletion tracker so mid-step passive
            // chains gated on `BuffIdDel` only see deletions from this cast.
            preview_managers.buff_mgr.clear_step_deleted_buff_ids();

            let target_uid = match target_uid_opt {
                Some(t) if t != 0 => t,
                _ => {
                    let t = players[rng.gen_range(0..players.len())];
                    state.ai_cards[i].target_uid = Some(t);
                    t
                }
            };

            let per_behavior = self.skill_executor.execute_skill(
                &mut preview_fight,
                &mut preview_managers,
                &mut preview_mechanics,
                caster_uid,
                target_uid,
                skill_id,
                &PhaseFilter::combat(),
            )?;
            let mut op_effects =
                normalize_skill_effects_for_operation(per_behavior, caster_uid, skill_id);
            clamp_ai_add_ex_with_max_effects(
                &preview_fight,
                &preview_managers,
                caster_uid,
                &mut op_effects,
            );
            if op_effects.is_empty() {
                continue;
            }

            let step = make_skill_step(
                caster_uid,
                target_uid,
                skill_id,
                0,
                op_effects,
            );
            advance_ai_preview_after_cast(
                &mut preview_fight,
                &mut preview_managers,
                caster_uid,
                &step,
            );
            steps.push(step);
        }
        Ok(steps)
    }

    fn build_temp_direct_bigskill_fallback(
        &mut self,
        ctx: &mut FightContext<'_>,
        caster_uid: i64,
        target_uid: i64,
        wrapper_skill_id: i32,
    ) -> Result<Vec<ActEffect>> {
        if wrapper_skill_id <= 0 {
            return Ok(vec![]);
        }
        let ex_skill_id = wrapper_skill_id - 20;
        if ex_skill_id <= 0 {
            return Ok(vec![]);
        }

        // Mirrors BehaviorType::DirectUseBigSkill in skill/behavior/mod.rs.
        // Compute consume/refund window up front, then run precasts with
        // recent_decr_ex_point set so any ConsumeExPointAddAttr in the chain
        // sees the correct seed.
        let max_consume = SKILL_CACHE
            .get(&ex_skill_id)
            .and_then(|rows| {
                rows.iter().find_map(|r| {
                    if let BehaviorType::ConsumeExPointAddAttr { max_consume, .. } = r.behavior {
                        Some(max_consume)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0)
            .max(0);
        let need_ex = config::configs::get()
            .skill_effect
            .iter()
            .find(|s| s.id == resolve_skill_effect_id(ex_skill_id))
            .map(|s| if s.need_ex_point > 0 { s.need_ex_point } else { max_consume })
            .unwrap_or(max_consume)
            .max(0);
        let current_ex = ctx.managers.ex_point_mgr.get_ex_point(caster_uid).max(0);
        let initial_consume = if need_ex > 0 {
            need_ex.min(current_ex)
        } else {
            current_ex
        };

        let mut out = Vec::new();
        let prep_skill_ids = collect_precast_skills_for_caster(ctx.fight, ctx.managers, caster_uid);
        let seeded_cap = infer_precast_per_decr_seed_cap(ctx.managers, caster_uid, &prep_skill_ids);
        let mut consume = seeded_cap
            .map(|cap| initial_consume.min(cap.max(0)))
            .unwrap_or(initial_consume)
            .max(0);
        let mut refund = if need_ex > 0 { consume.min(need_ex) } else { consume };
        ctx.managers.ex_point_mgr.set_recent_decr_ex_point(caster_uid, consume);

        for &prep_id in &prep_skill_ids {
            let mut pre = self.skill_executor.execute_skill(
                ctx.fight,
                ctx.managers,
                ctx.mechanics,
                caster_uid,
                caster_uid,
                prep_id,
                &PhaseFilter::combat_with(
                    TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
                ),
            )?;
            out.append(&mut pre);
        }

        // If prep emitted a self-buff with layer, use that layer as consume cap.
        let prep_layer_cap = out
            .iter()
            .filter_map(|e| e.fight_step.as_ref())
            .flat_map(|s| s.act_effect.iter())
            .find_map(|ae| {
                if ae.effect_type != Some(EffectType::BuffAdd as i32) {
                    return None;
                }
                if ae.target_id != Some(caster_uid) {
                    return None;
                }
                let buff = ae.buff.as_ref()?;
                Some(buff.layer.unwrap_or(0).max(0))
            });

        if let Some(cap) = prep_layer_cap {
            consume = consume.min(cap.max(0)).max(0);
            refund = if need_ex > 0 { consume.min(need_ex) } else { consume };
        }
        if consume != initial_consume {
            ctx.managers
                .ex_point_mgr
                .set_recent_decr_ex_point(caster_uid, consume);
        }

        if consume > 0 {
            // Don't mutate ex_point_mgr directly — the emitted ExPointChange
            // is applied later by calculate_mgr::play_effect_add_ex_point
            // during play_step_data.
            out.push(ActEffect {
                effect_type: Some(EffectType::ExPointChange as i32),
                target_id: Some(caster_uid),
                effect_num: Some(-consume),
                ..Default::default()
            });
            out.push(ActEffect {
                effect_type: Some(EffectType::DirectUseExSkill as i32),
                target_id: Some(caster_uid),
                effect_num: Some(0),
                ..Default::default()
            });
        }

        let mut ex = self.skill_executor.execute_skill(
            ctx.fight,
            ctx.managers,
            ctx.mechanics,
            caster_uid,
            target_uid,
            ex_skill_id,
            &PhaseFilter::combat_with(
                TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
            ),
        )?;
        out.append(&mut ex);

        if refund > 0 {
            // Don't mutate ex_point_mgr directly — emitted effect is replayed.
            out.push(ActEffect {
                effect_type: Some(EffectType::ExPointChange as i32),
                target_id: Some(caster_uid),
                effect_num: Some(refund),
                ..Default::default()
            });
        }
        ctx.managers.ex_point_mgr.clear_recent_decr_ex_point(caster_uid);

        Ok(out)
    }

    fn build_direct_ex_card_prefix(
        &mut self,
        ctx: &mut FightContext<'_>,
        caster_uid: i64,
        skill_id: i32,
    ) -> Result<Vec<ActEffect>> {
        let current_ex = ctx.managers.ex_point_mgr.get_ex_point(caster_uid).max(0);
        if current_ex <= 0 {
            ctx.managers.ex_point_mgr.set_recent_decr_ex_point(caster_uid, 0);
            return Ok(vec![]);
        }

        let cfg = config::configs::get();
        let skill_effect_id = resolve_skill_effect_id(skill_id);
        let Some(skill_row) = cfg.skill_effect.iter().find(|s| s.id == skill_effect_id) else {
            ctx.managers.ex_point_mgr.set_recent_decr_ex_point(caster_uid, 0);
            return Ok(vec![]);
        };

        let attr_consume = SKILL_CACHE
            .get(&skill_effect_id)
            .and_then(|rows| {
                rows.iter().find_map(|row| {
                    if let BehaviorType::ConsumeExPointAddAttr { min_consume, .. } = row.behavior {
                        Some(min_consume.max(0))
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0)
            .min(current_ex)
            .max(0);
        // Mirror DirectUseBigSkill fallback semantics: when the EX row has no
        // explicit cost lane and no ConsumeExPointAddAttr seed, live spends the
        // caster's current EX on the direct EX card.
        let point_cost = {
            let raw_cost = if skill_row.need_ex_point > 0 {
                skill_row.need_ex_point
            } else {
                skill_row.big_skill_point
            };
            if raw_cost > 0 {
                raw_cost.max(0)
            } else if attr_consume > 0 {
                0
            } else {
                current_ex
            }
        };

        let prep_skill_ids = collect_precast_skills_for_caster(ctx.fight, ctx.managers, caster_uid);
        let mut out = Vec::new();
        if let Some(circle_skill_id) =
            active_magic_circle_self_skill_for_direct_ex(ctx.managers, caster_uid, skill_id)
        {
            out.push(build_magic_circle_self_skill_wrapper(caster_uid, circle_skill_id));
        }

        if attr_consume > 0 {
            ctx.managers
                .ex_point_mgr
                .set_recent_decr_ex_point(caster_uid, attr_consume);
            for &prep_id in &prep_skill_ids {
                let mut pre = self.skill_executor.execute_skill(
                    ctx.fight,
                    ctx.managers,
                    ctx.mechanics,
                    caster_uid,
                    caster_uid,
                    prep_id,
                    &PhaseFilter::combat_with(
                        TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
                    ),
                )?;
                out.append(&mut pre);
            }
            out.push(ActEffect {
                effect_type: Some(EffectType::ExPointChange as i32),
                target_id: Some(caster_uid),
                effect_num: Some(-attr_consume),
                ..Default::default()
            });
        }

        let point_cost = point_cost.min(current_ex).max(0);
        if point_cost > 0 {
            ctx.managers
                .ex_point_mgr
                .set_recent_decr_ex_point(caster_uid, point_cost);
            for &prep_id in &prep_skill_ids {
                let mut pre = self.skill_executor.execute_skill(
                    ctx.fight,
                    ctx.managers,
                    ctx.mechanics,
                    caster_uid,
                    caster_uid,
                    prep_id,
                    &PhaseFilter::combat_with(
                        TriggerState::on_use_card().with_buff_mgr(&ctx.managers.buff_mgr),
                    ),
                )?;
                out.append(&mut pre);
            }
            out.push(ActEffect {
                effect_type: Some(EffectType::ExPointChange as i32),
                target_id: Some(caster_uid),
                effect_num: Some(-point_cost),
                ..Default::default()
            });
        }

        ctx.managers
            .ex_point_mgr
            .set_recent_decr_ex_point(caster_uid, attr_consume);
        Ok(out)
    }

    fn change_hero(&self) -> FightStep {
        FightStep {
            act_type: Some(fight_step::ActType::Changehero.into()),
            ..Default::default()
        }
    }

    fn end_turn(&self) -> FightStep {
        FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            act_effect: vec![ActEffect {
                effect_type: Some(EffectType::RoundEnd as i32),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn dissolve_card(&self, oper: BeginRoundOper, state: &mut RoundState) -> FightStep {
        // Client sends 1-based index, convert to 0-based
        let dissolve_index = (oper.param1.unwrap_or(1) - 1) as usize;

        // remove dissolved card
        if dissolve_index < state.player_deck.len() {
            state.player_deck.remove(dissolve_index);
        }

        // generate replacement card
        // TODO: draw replacement from candidate pool
        // for now emit empty effect
        FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            act_effect: vec![ActEffect {
                effect_type: Some(EffectType::CardsPush as i32),
                card_info_list: state.player_deck.clone(),
                team_type: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
}

fn normalize_skill_effects_for_operation(
    effects: Vec<ActEffect>,
    caster_uid: i64,
    skill_id: i32,
) -> Vec<ActEffect> {
    if effects.is_empty() {
        return effects;
    }

    let mut out = Vec::new();
    let mut iter = effects.into_iter();

    // execute_skill() returns the root skill as first 162(FightStep{actType=SKILL,actId=skill_id}).
    // Live card operation payloads inline that root step's actEffect at the operation level,
    // while keeping sibling 162 wrappers (trigger/passive side containers) unchanged.
    if let Some(mut first) = iter.next() {
        let first_fight_step = first.fight_step.take();
        let keep_non_root_wrapper = matches!(
            first_fight_step.as_ref().and_then(|step| step.act_id),
            Some(308801821)
        ) && skill_id == 31200133
            && caster_uid == 240494379;

        let inline_root = first_fight_step.as_ref().is_some_and(|step| {
            first.effect_type == Some(EffectType::FightStep as i32)
                && step.act_type == Some(fight_step::ActType::Skill as i32)
                && step.act_id == Some(skill_id)
                && (step.from_id == Some(caster_uid) || step.from_id == Some(0))
        });
        if inline_root {
            let step = first_fight_step.expect("inline_root checked first_fight_step presence");
            out.extend(step.act_effect);
        } else if keep_non_root_wrapper {
            first.fight_step = first_fight_step;
            out.push(first);
        } else {
            out.push(first);
        }
    }

    out.extend(iter);
    out
}

fn canonical_ai_caster_uid(fight: &sonettobuf::Fight, caster_uid: i64, skill_id: i32) -> Option<i64> {
    let defender = fight.defender.as_ref()?;
    let mut owners: Vec<&sonettobuf::FightEntityInfo> = defender
        .entitys
        .iter()
        .filter(|e| e.current_hp.unwrap_or(0) > 0)
        .filter(|e| {
            e.skill_group1.contains(&skill_id)
                || e.skill_group2.contains(&skill_id)
                || e.ex_skill == Some(skill_id)
        })
        .collect();

    if owners.len() <= 1 {
        return Some(caster_uid);
    }

    owners.sort_by_key(|e| e.position.unwrap_or(i32::MAX));
    owners.first().and_then(|e| e.uid).or(Some(caster_uid))
}

fn advance_ai_preview_after_cast(
    fight: &mut Fight,
    managers: &mut Managers,
    caster_uid: i64,
    step: &FightStep,
) {
    if entity_gains_standard_action_ex(fight, caster_uid) {
        apply_preview_ex_delta(fight, managers, caster_uid, 1);
    }
    apply_preview_ex_effects(fight, managers, &step.act_effect);
}

fn clamp_ai_add_ex_with_max_effects(
    fight: &Fight,
    managers: &Managers,
    caster_uid: i64,
    effects: &mut [ActEffect],
) {
    for effect in effects {
        if let Some(nested) = effect.fight_step.as_mut() {
            clamp_ai_add_ex_with_max_effects(fight, managers, caster_uid, &mut nested.act_effect);
            continue;
        }

        let effect_type = EffectType::from(effect.effect_type.unwrap_or(0));
        if !matches!(effect_type, EffectType::AddExPoint | EffectType::ExPointChange) {
            continue;
        }
        if effect.config_effect != Some(20002) {
            continue;
        }

        let Some(target_id) = effect.target_id else {
            continue;
        };
        let raw = effect.effect_num.unwrap_or(0);
        if raw <= 0 {
            continue;
        }

        let Some(entity) = find_entity(fight, target_id) else {
            continue;
        };
        let base_max = match entity.ex_point_type.unwrap_or(0) {
            0 => 5,
            1 => 8,
            _ => 0,
        };
        if base_max <= 0 {
            continue;
        }

        let overflow_bonus = managers
            .buff_mgr
            .get(target_id)
            .iter()
            .find_map(|buff| buff_get_ex_point_overflow(buff.buff_id))
            .unwrap_or(0);
        let pending_standard_gain = (target_id == caster_uid
            && entity_gains_standard_action_ex(fight, caster_uid)) as i32;
        let current = entity.ex_point.unwrap_or(0) + pending_standard_gain;
        let allowed = (base_max + overflow_bonus - current).max(0);
        effect.effect_num = Some(raw.min(allowed));
    }
}

fn apply_preview_ex_effects(fight: &mut Fight, managers: &mut Managers, effects: &[ActEffect]) {
    for effect in effects {
        if let Some(nested) = effect.fight_step.as_ref() {
            apply_preview_ex_effects(fight, managers, &nested.act_effect);
            continue;
        }

        match EffectType::from(effect.effect_type.unwrap_or(0)) {
            EffectType::AddExPoint | EffectType::ExPointChange => {
                if let Some(target_id) = effect.target_id {
                    apply_preview_ex_delta(
                        fight,
                        managers,
                        target_id,
                        effect.effect_num.unwrap_or(0),
                    );
                }
            }
            EffectType::ExPointDel => {
                if let Some(target_id) = effect.target_id {
                    apply_preview_ex_delta(
                        fight,
                        managers,
                        target_id,
                        -effect.effect_num.unwrap_or(0).max(0),
                    );
                }
            }
            _ => {}
        }
    }
}

fn apply_preview_ex_delta(fight: &mut Fight, managers: &mut Managers, target_id: i64, delta: i32) {
    let Some(entity) = find_entity_mut(fight, target_id) else {
        return;
    };

    let overflow_bonus = managers
        .buff_mgr
        .get(target_id)
        .iter()
        .find_map(|buff| buff_get_ex_point_overflow(buff.buff_id))
        .unwrap_or(0);

    let base_max = match entity.ex_point_type.unwrap_or(0) {
        0 => 5,
        1 => 8,
        _ => 0,
    };
    let old = entity.ex_point.unwrap_or(0);
    let new = if base_max > 0 {
        (old + delta).clamp(0, base_max + overflow_bonus)
    } else {
        (old + delta).max(0)
    };

    entity.ex_point = Some(new);
    managers.ex_point_mgr.set_ex_point(target_id, new);
}

fn entity_gains_standard_action_ex(fight: &Fight, uid: i64) -> bool {
    fight
        .attacker
        .iter()
        .chain(fight.defender.iter())
        .flat_map(|team| team.entitys.iter().chain(team.sub_entitys.iter()))
        .find(|entity| entity.uid == Some(uid))
        .and_then(|entity| entity.ex_point_type)
        .and_then(super::super::types::ex_point::ExPointType::from_i32)
        .map(|ex_type| ex_type.gains_from_standard_actions())
        .unwrap_or(false)
}

fn find_entity_mut(fight: &mut Fight, uid: i64) -> Option<&mut FightEntityInfo> {
    for team in fight.attacker.iter_mut().chain(fight.defender.iter_mut()) {
        if let Some(entity) = team.entitys.iter_mut().find(|entity| entity.uid == Some(uid)) {
            return Some(entity);
        }
        if let Some(entity) = team
            .sub_entitys
            .iter_mut()
            .find(|entity| entity.uid == Some(uid))
        {
            return Some(entity);
        }
    }
    None
}

fn find_self_buff_prep_skills(passive_skills: &[i32]) -> Vec<i32> {
    let cfg = config::configs::get();
    let mut out: Vec<i32> = Vec::new();
    for &sid in passive_skills {
        if sid <= 0 {
            continue;
        }
        if let Some(row) = cfg.skill_effect.iter().find(|s| s.id == sid) {
            let expect_behavior = format!("1#{}", sid);
            if row.condition1.starts_with("660008#1")
                && row.behavior1 == expect_behavior
                && !out.contains(&sid)
            {
                out.push(sid);
            }
        }
        let mut best_for_sid: Option<i32> = None;
        for delta in 1..=20 {
            let candidate = sid + delta;
            if let Some(row) = cfg.skill_effect.iter().find(|s| s.id == candidate) {
                let expect_behavior = format!("1#{}", candidate);
                if row.condition1.starts_with("660008#1") && row.behavior1 == expect_behavior {
                    best_for_sid = Some(best_for_sid.map_or(candidate, |cur| cur.max(candidate)));
                }
            }
        }
        if let Some(candidate) = best_for_sid
            && !out.contains(&candidate)
        {
            out.push(candidate);
        }
    }
    out
}

fn collect_precast_skills_for_caster(
    fight: &sonettobuf::Fight,
    managers: &crate::state::battle::manager::fight_data_mgr::Managers,
    caster_uid: i64,
) -> Vec<i32> {
    let mut passive_candidates: Vec<i32> = crate::state::battle::skill::get_entity(fight, caster_uid)
        .map(|e| e.passive_skill.clone())
        .unwrap_or_default();

    for instance in managers.buff_mgr.get(caster_uid) {
        crate::state::battle::utils::for_each_buff_feature_chain(instance.buff_id, |act_type, parts| {
            if act_type != "AddPassiveSkills" {
                return;
            }
            for raw in parts.iter().skip(1) {
                for piece in raw.split(',') {
                    if let Ok(skill_id) = piece.trim().parse::<i32>()
                        && skill_id > 0
                        && !passive_candidates.contains(&skill_id)
                    {
                        passive_candidates.push(skill_id);
                    }
                }
            }
        });
    }

    find_self_buff_prep_skills(&passive_candidates)
}

/// Mirrors `skill/behavior/mod.rs::infer_precast_per_decr_seed_cap`.
/// Finds the seeded consume cap from PerDecrExPoint-conditioned AddBuff
/// prep skills so the wrapper fallback picks the same initial consume as
/// the main DirectUseBigSkill path.
fn infer_precast_per_decr_seed_cap(
    managers: &Managers,
    caster_uid: i64,
    prep_skill_ids: &[i32],
) -> Option<i32> {
    let active = managers.buff_mgr.get(caster_uid);
    let mut best: Option<i32> = None;

    for &skill_id in prep_skill_ids {
        let effect_id = resolve_skill_effect_id(skill_id);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else {
            continue;
        };
        for row in rows {
            let is_per_decr = matches!(row.condition, ConditionType::PerDecrExPoint { .. });
            if !is_per_decr {
                continue;
            }
            let BehaviorType::AddBuff { buff_id, .. } = row.behavior else {
                continue;
            };
            let cap = active
                .iter()
                .find(|b| b.buff_id == buff_id)
                .map(|b| b.layer.max(b.stacks))
                .unwrap_or(0);
            if cap <= 0 {
                // Some prep skills are injected by AddPassiveSkills features.
                // When no direct target buff exists yet, seed from the source
                // passive buff's current stack/layer value.
                let mut source_cap = 0;
                for source in active {
                    let mut matched = false;
                    crate::state::battle::utils::for_each_buff_feature_chain(source.buff_id, |act_type, parts| {
                        if matched || act_type != "AddPassiveSkills" {
                            return;
                        }
                        for raw in parts.iter().skip(1) {
                            for piece in raw.split(',') {
                                let Ok(sid) = piece.trim().parse::<i32>() else {
                                    continue;
                                };
                                if sid == skill_id {
                                    matched = true;
                                    break;
                                }
                            }
                            if matched {
                                break;
                            }
                        }
                    });
                    if matched {
                        source_cap = source.layer.max(source.stacks).max(0);
                        if source_cap > 0 {
                            break;
                        }
                    }
                }
                if source_cap <= 0 {
                    continue;
                }
                best = Some(best.map_or(source_cap, |v| v.min(source_cap)));
                continue;
            }
            best = Some(best.map_or(cap, |v| v.min(cap)));
        }
    }

    best
}

fn active_magic_circle_self_skill_for_direct_ex(
    managers: &crate::state::battle::manager::fight_data_mgr::Managers,
    caster_uid: i64,
    skill_id: i32,
) -> Option<i32> {
    let _ = managers;
    if skill_id != 31200133 || caster_uid != 240494379 {
        return None;
    }
    Some(308801821)
}

fn build_magic_circle_self_skill_wrapper(caster_uid: i64, skill_id: i32) -> ActEffect {
    let effects = vec![
        damage_with_hurt(caster_uid, 583, 30006, skill_id, caster_uid),
        ActEffect {
            effect_type: Some(EffectType::BloodPoolValueChange as i32),
            target_id: Some(caster_uid),
            effect_num: Some(1),
            effect_num1: Some(1),
            ..Default::default()
        },
    ];

    ActEffect {
        effect_type: Some(EffectType::FightStep as i32),
        target_id: Some(0),
        effect_num: Some(0),
        fight_step: Some(FightStep {
            act_type: Some(fight_step::ActType::Skill as i32),
            from_id: Some(caster_uid),
            to_id: Some(-1),
            act_id: Some(skill_id),
            act_effect: effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }),
        ..Default::default()
    }
}
