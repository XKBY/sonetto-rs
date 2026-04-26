use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep, effect_type_enum::EffectType, fight_step};
use std::{collections::HashMap, sync::Mutex};

use crate::state::battle::context::FightContext;
use crate::state::battle::mechanics::shadowcloak;
use crate::state::battle::{
    buff_actions::blood_pool_ex::build_blood_pool_ex_point_step,
    buff_actions::raspberry::buff_get_raspberry_params,
    buff_actions::{magic_circle, nuodika, round_end},
    fight_step::{ActEffectBuilder, FightStepBuilder, effect_container_step, wrap_step},
    manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr, round_mgr::FightRoundMgr},
    passives::{collector::CollectedPassives, steps::build_passive_step},
    trigger::{combat::event_from_step, passes::build_belief_gain_step},
    utils::{damage_with_buff_act, find_entity, find_uid_by_hero_id},
};

const DAMAGE_PER_POINT: i32 = 3000;
const BASE_MAX: i32 = 24;
const PER_ALLY_BONUS: i32 = 16;
/// Skill 308801311 hosts the Semmelweis-class bloodtithe transition bundle.
const BLOODTITHE_TRANSITION_HOST_SKILL: i32 = 308801311;
/// Skill 31200193 hosts the Nautika-class psychube bundle.
const FAITH_PSYCHUBE_BUNDLE_HOST_SKILL: i32 = 31200193;

static GAINED: Lazy<Mutex<i32>> = Lazy::new(|| Mutex::new(0));

#[derive(Debug, Default, Clone, PartialEq)]
pub struct BloodtitheState {
    value: HashMap<i32, i32>,
    max: HashMap<i32, i32>,
    accumulator: HashMap<i32, i32>,
    pub initialized: bool,
    pub pending_effects: Vec<ActEffect>,
    pub moxie_threshold_tracker: HashMap<i32, i32>,
}

#[allow(dead_code)]
impl BloodtitheState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_bloodpool(&self) -> bool {
        self.initialized
    }

    pub fn recalc_max(&mut self, team_type: i32, enabler_count: i32) {
        let max = BASE_MAX + (enabler_count * PER_ALLY_BONUS);
        self.max.insert(team_type, max);

        let cur = self.value.entry(team_type).or_insert(0);
        if *cur > max {
            *cur = max;
        }

        tracing::info!(
            "Bloodtithe max recalculated: team={} enablers={} max={}",
            team_type,
            enabler_count,
            max
        );
    }

    pub fn on_hp_lost(&mut self, uid: i64, team_type: i32, hp_lost: i32) -> Option<i32> {
        let acc = self.accumulator.entry(team_type).or_insert(0);
        *acc += hp_lost;

        let max = *self.max.get(&team_type).unwrap_or(&BASE_MAX);
        let value = self.value.entry(team_type).or_insert(0);

        let mut gained = 0;
        set_gain(0);

        while *acc >= DAMAGE_PER_POINT && *value < max {
            *acc -= DAMAGE_PER_POINT;
            *value += 1;
            gained += 1;
            set_gain(gained);
            bloodtithe_add_to_pool(uid, gained);
        }

        if gained > 0 {
            tracing::info!(
                "Bloodtithe gained: team={} +{} => {}/{} (acc {}/{})",
                team_type,
                gained,
                *value,
                max,
                *acc,
                DAMAGE_PER_POINT
            );
            Some(gained)
        } else {
            None
        }
    }

    pub fn get_value(&self, team_type: i32) -> i32 {
        *self.value.get(&team_type).unwrap_or(&0)
    }

    pub fn get_max(&self, team_type: i32) -> i32 {
        *self.max.get(&team_type).unwrap_or(&BASE_MAX)
    }

    pub fn set_max(&mut self, team_type: i32, max: i32) {
        self.max.insert(team_type, max);
        let cur = self.value.entry(team_type).or_insert(0);
        if *cur > max {
            *cur = max;
        }
    }

    pub fn get_acc(&self, team_type: i32) -> i32 {
        *self.accumulator.get(&team_type).unwrap_or(&0)
    }

    pub fn set_value(&mut self, team_type: i32, value: i32) {
        let max = self.get_max(team_type);
        let capped_value = value.min(max);
        self.value.insert(team_type, capped_value);

        tracing::debug!(
            "Bloodtithe set_value: team={} value={} (capped at {})",
            team_type,
            value,
            capped_value
        );
    }

    pub fn add_initial_gain(&mut self, team_type: i32, amount: i32) {
        let current = self.get_value(team_type);
        let new_value = current + amount;
        self.set_value(team_type, new_value);

        set_gain(amount);

        tracing::info!(
            "Bloodtithe initial gain: team={} +{} => {}/{}",
            team_type,
            amount,
            self.get_value(team_type),
            self.get_max(team_type)
        );
    }

    pub fn consume_moxie_thresholds(&mut self, team_type: i32, threshold: i32) -> i32 {
        let total = self.get_value(team_type);
        let tracker = self.moxie_threshold_tracker.entry(team_type).or_insert(0);
        let new_moxie = (total / threshold) - (*tracker / threshold);
        if new_moxie > 0 {
            *tracker = (total / threshold) * threshold;
        }
        new_moxie.max(0)
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.max.clear();
        self.accumulator.clear();
    }

    pub fn reset_temporary_state(&mut self) {
        self.accumulator.clear();
        set_gain(0);
    }
}

pub(crate) fn build_round_transition_bloodtithe_steps(
    mgr: &FightRoundMgr,
    ctx: &mut FightContext<'_>,
    collected: &CollectedPassives,
) -> Vec<FightStep> {
    let mut out = Vec::new();

    let raspberry_step = ctx.mechanics.on_raspberry(
        ctx.fight,
        &ctx.managers.buff_mgr,
        &mut ctx.managers.ex_point_mgr,
    );
    if let Some(step) = ctx.mechanics.on_pre_raspberry() {
        out.push(step);
    }
    if let Some(step) = raspberry_step {
        let raspberry_event = event_from_step(
            ctx.fight,
            step.from_id.unwrap_or(0),
            step.to_id.unwrap_or(0),
            step.act_id.unwrap_or(0),
            &step.act_effect,
        );
        let shadow_step = shadowcloak::build_shadow_cloak_full_cap_step(ctx, &step);
        out.push(step);
        for &(team_type, gain) in &raspberry_event.bloodpool_gain_packets_by_team {
            if let Some(sync_step) = build_belief_gain_step(ctx.fight, team_type, gain) {
                out.push(sync_step);
            }
        }
        if let Some(shadow_step) = shadow_step {
            out.push(shadow_step);
        }
    }
    if let Some(step) = ctx.mechanics.on_post_raspberry(
        ctx.fight,
        &ctx.managers.buff_mgr,
        &ctx.managers.ex_point_mgr,
    ) {
        out.push(step);
    }

    let attacker_uids = collected.attacker_uids();
    let mut consume_blood_steps = build_passive_step(
        ctx,
        &attacker_uids,
        collected,
        &crate::state::battle::skill::PhaseFilter::consume_blood(),
    );
    for step in &mut consume_blood_steps {
        if step.act_type == Some(sonettobuf::fight_step::ActType::Skill as i32) {
            let circle_embeds = magic_circle::build_magic_circle_self_skill_embeds(
                ctx,
                &step.clone(),
                step.from_id.unwrap_or(0),
            );
            if !circle_embeds.is_empty() {
                let insert_at =
                    crate::state::battle::steps::trigger_embed::find_trigger_insert_index(
                        &step.act_effect,
                    );
                step.act_effect.splice(insert_at..insert_at, circle_embeds);
            }
        }
    }
    out.extend(consume_blood_steps);

    out.extend(round_end::build_round_end_use_skill_to_enemy_steps(
        ctx, collected,
    ));

    let nuodika_steps = nuodika::build_nuodika_channel_steps(mgr, ctx, &out, collected);
    out.extend(nuodika_steps);

    if let Some(step) = build_blood_pool_ex_point_step(
        &mut ctx.mechanics.bloodtithe,
        ctx.fight,
        &ctx.managers.buff_mgr,
        &mut ctx.managers.ex_point_mgr,
    ) && !step.act_effect.is_empty()
    {
        out.push(step);
    }

    merge_bloodtithe_transition_hosts(ctx.fight, &mut out);

    out
}

fn merge_bloodtithe_transition_hosts(fight: &Fight, out: &mut Vec<FightStep>) {
    let semm_host_idx = find_transition_host_index(out, BLOODTITHE_TRANSITION_HOST_SKILL);
    let naut_host_idx = find_transition_host_index(out, FAITH_PSYCHUBE_BUNDLE_HOST_SKILL);

    if semm_host_idx.is_none() && naut_host_idx.is_none() {
        return;
    }

    let mut semm_embeds = Vec::new();
    let mut naut_embeds = Vec::new();
    let mut remove_indices = Vec::new();

    for idx in 0..out.len() {
        if Some(idx) == semm_host_idx || Some(idx) == naut_host_idx {
            continue;
        }

        let Some(host) =
            classify_transition_ancillary_step(fight, &out[idx], idx, semm_host_idx, naut_host_idx)
        else {
            continue;
        };

        match host {
            TransitionHost::Semmelweis if semm_host_idx.is_some() => {
                semm_embeds.push(wrap_step(out[idx].clone()));
                remove_indices.push(idx);
            }
            TransitionHost::Nautika if naut_host_idx.is_some() => {
                naut_embeds.push(wrap_step(out[idx].clone()));
                remove_indices.push(idx);
            }
            _ => {}
        }
    }

    if let Some(idx) = semm_host_idx
        && !semm_embeds.is_empty()
    {
        out[idx].act_effect.extend(semm_embeds);
    }
    if let Some(idx) = naut_host_idx
        && !naut_embeds.is_empty()
    {
        out[idx].act_effect.extend(naut_embeds);
    }

    if remove_indices.is_empty() {
        return;
    }

    remove_indices.sort_unstable();
    remove_indices.dedup();
    for idx in remove_indices.into_iter().rev() {
        out.remove(idx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransitionHost {
    Semmelweis,
    Nautika,
}

fn find_transition_host_index(out: &[FightStep], host_act_id: i32) -> Option<usize> {
    out.iter()
        .position(|step| transition_host_act_id(step) == Some(host_act_id))
}

fn transition_host_act_id(step: &FightStep) -> Option<i32> {
    if step.act_type != Some(fight_step::ActType::Effect as i32) {
        return None;
    }

    let wrapped = step.act_effect.first()?;
    if wrapped.effect_type != Some(162) {
        return None;
    }

    let host = wrapped.fight_step.as_ref()?;
    if host.act_type != Some(fight_step::ActType::Effect as i32) {
        return None;
    }

    host.act_id
}

fn classify_transition_ancillary_step(
    fight: &Fight,
    step: &FightStep,
    _idx: usize,
    _semm_host_idx: Option<usize>,
    _naut_host_idx: Option<usize>,
) -> Option<TransitionHost> {
    let ownership = inspect_step_ownership(fight, step);

    if ownership.skill_from_nautika {
        return Some(TransitionHost::Nautika);
    }
    if ownership.skill_from_semmelweis {
        return Some(TransitionHost::Semmelweis);
    }

    if ownership.from_nautika || ownership.to_nautika {
        return Some(TransitionHost::Nautika);
    }
    if ownership.from_semmelweis || ownership.to_semmelweis || ownership.targets_semmelweis {
        return Some(TransitionHost::Semmelweis);
    }
    if ownership.targets_nautika {
        return Some(TransitionHost::Nautika);
    }

    None
}

#[derive(Default)]
struct StepOwnership {
    from_semmelweis: bool,
    from_nautika: bool,
    to_semmelweis: bool,
    to_nautika: bool,
    targets_semmelweis: bool,
    targets_nautika: bool,
    skill_from_semmelweis: bool,
    skill_from_nautika: bool,
}

fn inspect_step_ownership(fight: &Fight, step: &FightStep) -> StepOwnership {
    let semmelweis_uid = find_uid_by_hero_id(fight, 3088);
    let nautika_uid = find_uid_by_hero_id(fight, 3120);
    let mut ownership = StepOwnership::default();
    collect_step_ownership(step, semmelweis_uid, nautika_uid, &mut ownership);
    ownership
}

fn collect_step_ownership(
    step: &FightStep,
    semmelweis_uid: Option<i64>,
    nautika_uid: Option<i64>,
    ownership: &mut StepOwnership,
) {
    if step.from_id.is_some() && step.from_id == semmelweis_uid {
        ownership.from_semmelweis = true;
    }
    if step.from_id.is_some() && step.from_id == nautika_uid {
        ownership.from_nautika = true;
    }

    if step.to_id.is_some() && step.to_id == semmelweis_uid {
        ownership.to_semmelweis = true;
    }
    if step.to_id.is_some() && step.to_id == nautika_uid {
        ownership.to_nautika = true;
    }

    if step.act_type == Some(fight_step::ActType::Skill as i32) {
        if step.from_id.is_some() && step.from_id == semmelweis_uid {
            ownership.skill_from_semmelweis = true;
        }
        if step.from_id.is_some() && step.from_id == nautika_uid {
            ownership.skill_from_nautika = true;
        }
    }

    for effect in &step.act_effect {
        if effect.target_id.is_some() && effect.target_id == semmelweis_uid {
            ownership.targets_semmelweis = true;
        }
        if effect.target_id.is_some() && effect.target_id == nautika_uid {
            ownership.targets_nautika = true;
        }

        if let Some(child) = effect.fight_step.as_ref() {
            collect_step_ownership(child, semmelweis_uid, nautika_uid, ownership);
        }
    }
}

pub fn bloodtithe_add_to_pool(target_uid: i64, new_total: i32) -> ActEffect {
    ActEffectBuilder::bloodpool_value_change(target_uid, 1, new_total)
}

pub fn set_gain(value: i32) {
    *GAINED.lock().unwrap() = value;
}

pub fn bloodtithe_max_change(amount: i32, change_type: i32) -> ActEffect {
    ActEffectBuilder::bloodpool_max_change(change_type, amount)
}

pub fn bloodtithe_value_change(target_uid: i64, amount: i32, change_type: i32) -> ActEffect {
    ActEffectBuilder::bloodpool_value_change(target_uid, change_type, amount)
}

impl BloodtitheState {
    pub fn bloodpool_init_step(&self) -> Option<FightStep> {
        if !self.initialized {
            return None;
        }
        Some(
            FightStepBuilder::effect()
                .with_many(vec![
                    ActEffectBuilder::new(EffectType::Bloodpoolmaxcreate as i32, 0)
                        .effect_num(1)
                        .build(),
                    ActEffectBuilder::bloodpool_max_change(1, 57),
                ])
                .build(),
        )
    }

    pub fn bloodtithe_sync_step(&self) -> Option<FightStep> {
        if !self.initialized {
            return None;
        }
        let value = self.get_value(1);
        if value == 0 {
            return None;
        }
        Some(
            FightStepBuilder::effect()
                .with(ActEffectBuilder::bloodpool_max_change(1, value))
                .build(),
        )
    }

    pub fn raspberry_step(
        &mut self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &mut ExPointMgr,
        shadow_cloak: &mut super::shadowcloak::ShadowCloakState,
    ) -> Option<FightStep> {
        if !self.initialized {
            return None;
        }

        let all_uids: Vec<i64> = fight
            .attacker
            .iter()
            .chain(fight.defender.iter())
            .flat_map(|side| {
                side.entitys
                    .iter()
                    .chain(side.sub_entitys.iter())
                    .filter(|e| e.position.unwrap_or(-1) > 0)
                    .filter_map(|e| e.uid)
            })
            .collect();

        let mut outer_effects: Vec<ActEffect> = Vec::new();

        for uid in all_uids {
            for instance in buff_mgr.get(uid) {
                let Some((act_id, rate_permille)) = buff_get_raspberry_params(instance.buff_id)
                else {
                    continue;
                };
                let current_hp = ex_point_mgr.get_hp(uid);
                let caster_uid = instance.from_uid;
                let damage = current_hp * rate_permille / 1000;
                if damage == 0 {
                    continue;
                }

                let is_shadow_cloak_slave = {
                    let cfg = config::configs::get();
                    cfg.skill_buff
                        .iter()
                        .find(|b| b.id == instance.buff_id)
                        .map(|b| {
                            b.type_id
                                == crate::state::battle::mechanics::shadowcloak::SHADOW_CLOAK_ACCUMULATOR_BUFF_ID
                        })
                        .unwrap_or(false)
                };

                if is_shadow_cloak_slave && shadow_cloak.is_active() {
                    shadow_cloak.add(uid, damage);
                }

                let entity = find_entity(fight, uid);
                let mut effects = vec![damage_with_buff_act(
                    uid,
                    damage,
                    act_id,
                    caster_uid,
                    instance.uid,
                )];

                if let Some(team_type) = entity.and_then(|e| e.team_type)
                    && let Some(gained) = self.on_hp_lost(uid, team_type, damage)
                {
                    if let Some(nautika_uid) = find_nautika_uid(fight, team_type) {
                        ex_point_mgr.add_ex_point(nautika_uid, gained);
                        effects.push(ActEffectBuilder::ex_point_change(nautika_uid, gained));
                    }
                    effects.push(bloodtithe_add_to_pool(uid, gained));
                }

                outer_effects.push(wrap_step(effect_container_step(
                    caster_uid,
                    uid,
                    instance.buff_id,
                    effects,
                )));
            }
        }

        if outer_effects.is_empty() {
            return None;
        }
        Some(FightStepBuilder::effect().with_many(outer_effects).build())
    }
}

fn find_nautika_uid(fight: &Fight, team_type: i32) -> Option<i64> {
    let side = if team_type == 1 {
        fight.attacker.as_ref()
    } else {
        fight.defender.as_ref()
    };
    side?
        .entitys
        .iter()
        .find(|e| e.model_id == Some(3120))
        .and_then(|e| e.uid)
}
