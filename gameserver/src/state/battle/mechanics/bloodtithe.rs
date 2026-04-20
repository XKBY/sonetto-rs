use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep, effect_type_enum::EffectType, fight_step};
use std::{collections::HashMap, sync::Mutex};

use crate::state::battle::{
    fight_step::FightStepBuilder,
    manager::{buff_mgr::BuffMgr, ex_point_mgr::ExPointMgr, round_mgr::FightRoundMgr},
    passives::{collector::CollectedPassives, steps::build_passive_step},
    trigger::{combat::event_from_step, passes::build_belief_gain_step},
    utils::{
        build_blood_pool_ex_point_step, buff_get_blood_pool_ex_point_params,
        buff_get_raspberry_params, damage_with_buff_act, find_entity,
    },
};
use crate::state::battle::context::FightContext;
use crate::state::battle::mechanics::{nuodika, round_end, shadowcloak};

const DAMAGE_PER_POINT: i32 = 3000;
const BASE_MAX: i32 = 24;
const PER_ALLY_BONUS: i32 = 16;

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
    let consume_blood_steps = build_passive_step(
        ctx,
        &attacker_uids,
        collected,
        &crate::state::battle::skill::PhaseFilter::consume_blood(),
    );
    out.extend(consume_blood_steps);

    out.extend(round_end::build_round_end_use_skill_to_enemy_steps(ctx));

    let nuodika_steps = nuodika::build_nuodika_channel_steps(mgr, ctx, &out);
    out.extend(nuodika_steps);

    if let Some(mut step) = build_blood_pool_ex_point_step(
        &mut ctx.mechanics.bloodtithe,
        ctx.fight,
        &ctx.managers.buff_mgr,
        &mut ctx.managers.ex_point_mgr,
    ) {
        if !step.act_effect.is_empty() {
            out.push(step);
        }
    }

    out
}

pub fn bloodtithe_add_to_pool(target_uid: i64, new_total: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
        target_id: Some(target_uid),
        effect_num: Some(1),
        effect_num1: Some(new_total),
        ..Default::default()
    }
}

pub fn set_gain(value: i32) {
    *GAINED.lock().unwrap() = value;
}

pub fn bloodtithe_max_change(amount: i32, change_type: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Bloodpoolmaxchange as i32),
        target_id: Some(0),
        effect_num: Some(change_type),
        effect_num1: Some(amount),
        ..Default::default()
    }
}

pub fn bloodtithe_value_change(target_uid: i64, amount: i32, change_type: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
        target_id: Some(target_uid),
        effect_num: Some(change_type),
        effect_num1: Some(amount),
        ..Default::default()
    }
}

impl BloodtitheState {
    pub fn bloodpool_init_step(&self) -> Option<FightStep> {
        if !self.initialized {
            return None;
        }
        Some(
            FightStepBuilder::effect()
                .with_many(vec![
                    ActEffect {
                        effect_type: Some(EffectType::Bloodpoolmaxcreate as i32),
                        effect_num: Some(1),
                        target_id: Some(0),
                        ..Default::default()
                    },
                    ActEffect {
                        effect_type: Some(EffectType::Bloodpoolmaxchange as i32),
                        effect_num: Some(1),
                        effect_num1: Some(57),
                        target_id: Some(0),
                        ..Default::default()
                    },
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
                .with(ActEffect {
                    effect_type: Some(EffectType::Bloodpoolmaxchange as i32),
                    effect_num: Some(1),
                    effect_num1: Some(value),
                    target_id: Some(0),
                    ..Default::default()
                })
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
                        effects.push(ActEffect {
                            effect_type: Some(EffectType::Expointchange as i32),
                            target_id: Some(nautika_uid),
                            effect_num: Some(gained),
                            ..Default::default()
                        });
                    }
                    effects.push(bloodtithe_add_to_pool(uid, gained));
                }

                outer_effects.push(ActEffect {
                    effect_type: Some(EffectType::Fightstep as i32),
                    target_id: Some(0),
                    fight_step: Some(FightStep {
                        act_type: Some(fight_step::ActType::Effect.into()),
                        from_id: Some(caster_uid),
                        to_id: Some(uid),
                        act_id: Some(instance.buff_id),
                        act_effect: effects,
                        card_index: Some(0),
                        support_hero_id: Some(0),
                        fake_timeline: Some(false),
                        real_skill_type: Some(0),
                        real_skin_id: Some(0),
                    }),
                    ..Default::default()
                });
            }
        }

        if outer_effects.is_empty() {
            return None;
        }
        Some(FightStepBuilder::effect().with_many(outer_effects).build())
    }

    pub fn blood_pool_ex_point_step(
        &self,
        fight: &Fight,
        buff_mgr: &BuffMgr,
        ex_point_mgr: &mut ExPointMgr,
    ) -> Option<FightStep> {
        if !self.initialized {
            return None;
        }
        let bloodtithe_value = self.get_value(1);
        if bloodtithe_value == 0 {
            return None;
        }

        let uids: Vec<i64> = fight
            .attacker
            .as_ref()
            .map(|a| a.entitys.iter().filter_map(|e| e.uid).collect())
            .unwrap_or_default();

        let mut outer_effects: Vec<ActEffect> = Vec::new();

        for uid in uids {
            for instance in buff_mgr.get(uid) {
                let Some((threshold, amount)) =
                    buff_get_blood_pool_ex_point_params(instance.buff_id)
                else {
                    continue;
                };
                let gain = (bloodtithe_value / threshold) * amount;
                if gain == 0 {
                    continue;
                }
                ex_point_mgr.add_ex_point(uid, gain);

                outer_effects.push(ActEffect {
                    effect_type: Some(EffectType::Fightstep as i32),
                    target_id: Some(0),
                    fight_step: Some(FightStep {
                        act_type: Some(fight_step::ActType::Effect.into()),
                        from_id: Some(uid),
                        to_id: Some(uid),
                        act_id: Some(instance.buff_id),
                        act_effect: vec![
                            ActEffect {
                                effect_type: Some(0),
                                effect_num: Some(instance.buff_id),
                                buff_act_id: Some(1021),
                                target_id: Some(uid),
                                ..Default::default()
                            },
                            ActEffect {
                                effect_type: Some(EffectType::Expointchange as i32),
                                effect_num: Some(gain),
                                target_id: Some(uid),
                                ..Default::default()
                            },
                        ],
                        card_index: Some(0),
                        support_hero_id: Some(0),
                        fake_timeline: Some(false),
                        real_skill_type: Some(0),
                        real_skin_id: Some(0),
                    }),
                    ..Default::default()
                });
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
