use super::super::{
    context::FightContext,
    event_queue::{BattleEvent, EventContext, EventQueue, drain_to_fight_steps},
    fight_step::split_step_by_effect_limit,
    manager::{
        buff_mgr::{BuffMgr, observe_explicit_buff_uid_for_target},
        calculate_mgr::FightCalculateDataMgr,
        entity_mgr::FightEntityDataMgr,
        ex_point_mgr::{ExPointMgr, build_ex_point_info, sync_to_fight},
        wave_mgr::WaveMgr,
    },
    mechanics::Mechanics,
    passives::run_battle_start,
};
use super::round_mgr::seed_entry_max_hp_from_fight;

use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::{BuffInfo, CardInfo, Fight, FightExPointInfo, FightRound, FightStep};

use crate::state::battle::{
    buff_actions::{blood_pool_ex::seed_blood_pool_ex_tracker, raspberry::BUFF_ACT_ID_RASPBERRY},
    heroes::rubuska,
    mechanics::bloodtithe::BloodtitheState,
    types::effects::EffectType,
};

#[derive(Debug, Clone, Default)]
pub struct Managers {
    pub entity_mgr: FightEntityDataMgr,
    pub calculate_mgr: FightCalculateDataMgr,
    pub buff_mgr: BuffMgr,
    pub ex_point_mgr: ExPointMgr,
    pub wave_mgr: WaveMgr,
}

impl Managers {
    pub fn new(fight: &Fight) -> Self {
        Self {
            entity_mgr: FightEntityDataMgr::new(fight),
            calculate_mgr: FightCalculateDataMgr::new(fight),
            buff_mgr: BuffMgr::new(),
            ex_point_mgr: ExPointMgr::new(),
            wave_mgr: WaveMgr::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FightDataMgr {
    fight: Fight,
    pub pre_fight: Option<Fight>,
    mechanics: Mechanics,
    pub managers: Managers,
}

impl FightDataMgr {
    pub fn new(fight: Fight) -> Self {
        seed_entry_max_hp_from_fight(&fight);
        let mechanics = Mechanics::new();
        let pre_fight = Some(fight.clone());
        Self {
            managers: Managers::new(&fight),
            pre_fight,
            fight,
            mechanics,
        }
    }

    #[inline]
    pub fn fight(&self) -> &Fight {
        &self.fight
    }

    #[inline]
    pub fn get_fight(&self) -> &Fight {
        &self.fight
    }

    // Replay-bootstrap surface consumed by battle_gen; clippy can't see the
    // cross-crate callers so silence the per-method dead_code lint here.
    #[allow(dead_code)]
    #[inline]
    pub fn fight_mut(&mut self) -> &mut Fight {
        &mut self.fight
    }

    pub fn ctx(&mut self) -> FightContext<'_> {
        FightContext::new(&mut self.fight, &mut self.managers, &mut self.mechanics)
    }

    pub fn ctx_with_rng<'a>(&'a mut self, rng: &mut StdRng) -> FightContext<'a> {
        self.ctx().with_rng(rng)
    }

    pub fn build_initial_round(
        &mut self,
        battle_id: i32,
        player_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
    ) -> Result<FightRound> {
        // init ex_point_mgr from fight state
        self.managers.ex_point_mgr.init(&self.fight);

        let mut steps: Vec<FightStep> = Vec::new();
        let passive_steps = {
            let seed = self.fight.cur_round.unwrap_or(0) as u64;
            let mut rng = StdRng::seed_from_u64(seed);
            let mut ctx = self.ctx_with_rng(&mut rng);
            run_battle_start(&mut ctx, battle_id)
        };
        steps.extend(passive_steps);
        steps = steps
            .into_iter()
            .flat_map(split_step_by_effect_limit)
            .collect();

        for (uid, hp) in &self.managers.ex_point_mgr.current_hp {
            tracing::warn!("post-passive hp: uid={} hp={}", uid, hp);
        }

        sync_to_fight(&mut self.fight, &self.managers.ex_point_mgr);

        // sync HP into pre_fight so fight.entity.current_hp matches ex_point_info.current_hp
        // other fields (ex_point, moxie) stay at original values for client initialization
        if let Some(pre) = self.pre_fight.as_mut() {
            for side in [pre.attacker.as_mut(), pre.defender.as_mut()]
                .into_iter()
                .flatten()
            {
                for e in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                    if let Some(uid) = e.uid {
                        e.current_hp = Some(self.managers.ex_point_mgr.get_hp(uid));
                    }
                }
            }
        }

        self.managers.entity_mgr.rebuild_cache(&self.fight);
        self.managers.calculate_mgr.update_cache(&self.fight);

        let act_point = self
            .fight
            .attacker
            .as_ref()
            .map_or(3, |a| a.entitys.len() as i32);

        // build ex_point_info before moving fields
        for e in self.fight.attacker.iter().flat_map(|t| t.entitys.iter()) {
            let uid = e.uid.unwrap_or(0);
            tracing::warn!(
                "pre-build uid={} mgr_hp={}",
                uid,
                self.managers.ex_point_mgr.get_hp(uid)
            );
        }

        let ex_point_info = build_ex_point_info(&self.fight, &self.managers.ex_point_mgr);

        let hero_sp_attributes = self
            .managers
            .calculate_mgr
            .build_hero_sp_attributes(&self.fight);

        let skill_infos = self
            .fight
            .attacker
            .as_ref()
            .map(|a| a.skill_infos.clone())
            .unwrap_or_default();

        let team_a_cards1 = player_deck;

        Ok(FightRound {
            fight_step: steps,
            act_point: Some(act_point),
            is_finish: Some(false),
            move_num: Some(0),
            ex_point_info,
            ai_use_cards: ai_deck,
            power: Some(20),
            skill_infos,
            before_cards1: vec![],
            team_a_cards1,
            before_cards2: vec![],
            team_a_cards2: vec![],
            next_round_begin_step: vec![],
            use_card_list: vec![],
            cur_round: Some(1),
            hero_sp_attributes,
            last_change_hero_uid: Some(0),
        })
    }

    #[allow(dead_code)]
    pub fn seed_replay_state(
        &mut self,
        initial_round: &FightRound,
        ex_point_info: &[FightExPointInfo],
    ) -> Result<()> {
        self.managers.ex_point_mgr.init(&self.fight);
        self.mechanics.init(&self.fight);

        for step in &initial_round.fight_step {
            self.managers
                .calculate_mgr
                .play_step_data(
                    step,
                    &mut self.fight,
                    &mut self.mechanics.bloodtithe,
                    &mut self.managers.buff_mgr,
                    &mut self.managers.ex_point_mgr,
                )
                .map_err(anyhow::Error::msg)?;
        }

        if !ex_point_info.is_empty() {
            let by_uid: std::collections::HashMap<i64, &FightExPointInfo> = ex_point_info
                .iter()
                .filter_map(|info| info.uid.map(|uid| (uid, info)))
                .collect();

            for side in [&mut self.fight.attacker, &mut self.fight.defender]
                .into_iter()
                .flatten()
            {
                for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                    let uid = entity.uid.unwrap_or(0);
                    let Some(info) = by_uid.get(&uid) else {
                        continue;
                    };

                    let ex_point = info.ex_point.unwrap_or(0);
                    entity.ex_point = Some(ex_point);
                    self.managers.ex_point_mgr.set_ex_point(uid, ex_point);

                    if let Some(current_hp) = info.current_hp {
                        entity.current_hp = Some(current_hp);
                        self.managers.ex_point_mgr.set_hp(uid, current_hp);
                    }
                }
            }
        }

        self.managers.entity_mgr.rebuild_cache(&self.fight);
        self.managers.calculate_mgr.update_cache(&self.fight);
        // Replay seeding mutates max HP during initial-round step playback.
        // Refresh mechanics after that replay so Rubuska Shadow Cloak reads
        // the effective post-bootstrap baseline rather than the raw fight payload.
        self.mechanics.init(&self.fight);
        // Some buff-act state (e.g. Kakania's Empathy `actCommonParams`)
        // only lives in `BuffMgr` after the initial-round replay; the
        // entity buff snapshot in `fight` doesn't carry it forward. Pull
        // it into the mechanics caches so cumulative totals like the
        // Empathy injury bank don't regress between rounds.
        self.mechanics.sync_from_buff_mgr(&self.managers.buff_mgr);
        self.reseed_bloodtithe_from_round(initial_round);
        self.reseed_shadow_cloak_from_round(initial_round);
        seed_blood_pool_ex_tracker(
            &self.mechanics.bloodtithe,
            &self.fight,
            &self.managers.buff_mgr,
        );
        Ok(())
    }

    #[allow(dead_code)]
    pub fn seed_replay_bloodtithe_from_effects(&mut self, effects: &[(i32, i32, i32)]) {
        let mut rebuilt = BloodtitheState::new();
        for &(effect_type, team_type, amount) in effects {
            match EffectType::from(effect_type) {
                EffectType::BloodPoolMaxCreate => {
                    rebuilt.initialized = true;
                }
                EffectType::BloodPoolMaxChange => {
                    rebuilt.initialized = true;
                    let current_max = rebuilt.get_max(team_type);
                    if amount > current_max {
                        let mut queue = EventQueue::new();
                        queue.push(BattleEvent::BloodpoolMaxChange {
                            team_type,
                            max: current_max.max(amount),
                        });
                        let mut local_fight = Fight::default();
                        let mut local_buff_mgr = BuffMgr::new();
                        let mut local_ex_point_mgr = ExPointMgr::new();
                        let mut event_ctx = EventContext {
                            fight: &mut local_fight,
                            buff_mgr: &mut local_buff_mgr,
                            ex_point_mgr: &mut local_ex_point_mgr,
                            bloodtithe: &mut rebuilt,
                        };
                        let _ = drain_to_fight_steps(queue.drain(), &mut event_ctx);
                    }
                }
                EffectType::BloodPoolValueChange => {
                    rebuilt.initialized = true;
                    let current = rebuilt.get_value(team_type);
                    rebuilt.set_value(team_type, (current + amount).max(0));
                }
                _ => {}
            }
        }
        if rebuilt.initialized {
            self.mechanics.bloodtithe = rebuilt;
            seed_blood_pool_ex_tracker(
                &self.mechanics.bloodtithe,
                &self.fight,
                &self.managers.buff_mgr,
            );
        }
    }

    #[allow(dead_code)]
    pub fn seed_replay_buffs_from_effects(
        &mut self,
        effects: &[(i32, i64, i64, i32, i32, i64, i32)],
    ) {
        if effects.is_empty() {
            return;
        }

        for &(buff_id, target_uid, from_uid, count, layer, buff_uid, duration) in effects {
            if buff_id <= 0 || target_uid == 0 || buff_uid <= 0 {
                continue;
            }

            observe_explicit_buff_uid_for_target(target_uid, buff_uid);
            self.managers
                .buff_mgr
                .add_with_uid(target_uid, buff_id, from_uid, 0, count, layer, buff_uid);
            let _ = self
                .managers
                .buff_mgr
                .set_instance_duration(target_uid, buff_uid, duration);

            let mut seeded = false;
            for side in [&mut self.fight.attacker, &mut self.fight.defender]
                .into_iter()
                .flatten()
            {
                for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                    if entity.uid != Some(target_uid) {
                        continue;
                    }

                    if let Some(existing) =
                        entity.buffs.iter_mut().find(|b| b.uid == Some(buff_uid))
                    {
                        existing.buff_id = Some(buff_id);
                        existing.from_uid = Some(from_uid);
                        existing.count = Some(count);
                        existing.layer = Some(layer);
                        existing.duration = Some(duration);
                    } else {
                        entity.buffs.push(BuffInfo {
                            uid: Some(buff_uid),
                            buff_id: Some(buff_id),
                            from_uid: Some(from_uid),
                            count: Some(count),
                            layer: Some(layer),
                            duration: Some(duration),
                            ..Default::default()
                        });
                    }
                    seeded = true;
                    break;
                }
                if seeded {
                    break;
                }
            }
        }
    }

    #[allow(dead_code)]
    fn reseed_bloodtithe_from_round(&mut self, round: &FightRound) {
        #[derive(Clone, Copy)]
        struct Frame<'a> {
            step: &'a FightStep,
            next_effect: usize,
        }

        let mut rebuilt = BloodtitheState::new();
        let mut stack: Vec<Frame<'_>> = round
            .fight_step
            .iter()
            .rev()
            .map(|step| Frame {
                step,
                next_effect: 0,
            })
            .collect();

        while let Some(frame) = stack.last_mut() {
            if frame.next_effect >= frame.step.act_effect.len() {
                stack.pop();
                continue;
            }

            let effect = &frame.step.act_effect[frame.next_effect];
            frame.next_effect += 1;

            if let Some(nested) = effect.fight_step.as_ref() {
                stack.push(Frame {
                    step: nested,
                    next_effect: 0,
                });
                continue;
            }

            let team_type = effect.effect_num.or(effect.team_type).unwrap_or(1);
            match EffectType::from(effect.effect_type.unwrap_or(0)) {
                EffectType::BloodPoolMaxCreate => {
                    rebuilt.initialized = true;
                }
                EffectType::BloodPoolMaxChange => {
                    rebuilt.initialized = true;
                    let next_max = effect.effect_num1.unwrap_or(0);
                    let current_max = rebuilt.get_max(team_type);
                    if next_max > current_max {
                        let mut queue = EventQueue::new();
                        queue.push(BattleEvent::BloodpoolMaxChange {
                            team_type,
                            max: current_max.max(next_max),
                        });
                        let mut local_fight = Fight::default();
                        let mut local_buff_mgr = BuffMgr::new();
                        let mut local_ex_point_mgr = ExPointMgr::new();
                        let mut event_ctx = EventContext {
                            fight: &mut local_fight,
                            buff_mgr: &mut local_buff_mgr,
                            ex_point_mgr: &mut local_ex_point_mgr,
                            bloodtithe: &mut rebuilt,
                        };
                        let _ = drain_to_fight_steps(queue.drain(), &mut event_ctx);
                    }
                }
                EffectType::BloodPoolValueChange => {
                    rebuilt.initialized = true;
                    let delta = effect.effect_num1.unwrap_or(0);
                    let current = rebuilt.get_value(team_type);
                    rebuilt.set_value(team_type, (current + delta).max(0));
                }
                _ => {}
            }
        }

        if rebuilt.initialized {
            self.mechanics.bloodtithe = rebuilt;
        }
    }

    #[allow(dead_code)]
    fn reseed_shadow_cloak_from_round(&mut self, round: &FightRound) {
        #[derive(Clone, Copy)]
        struct Frame<'a> {
            step: &'a FightStep,
            next_effect: usize,
        }

        let mut seeded_max = 0;
        let mut stack: Vec<Frame<'_>> = round
            .fight_step
            .iter()
            .rev()
            .map(|step| Frame {
                step,
                next_effect: 0,
            })
            .collect();

        while let Some(frame) = stack.last_mut() {
            if frame.next_effect >= frame.step.act_effect.len() {
                stack.pop();
                continue;
            }

            let effect = &frame.step.act_effect[frame.next_effect];
            frame.next_effect += 1;

            if let Some(nested) = effect.fight_step.as_ref() {
                stack.push(Frame {
                    step: nested,
                    next_effect: 0,
                });
                continue;
            }

            if EffectType::from(effect.effect_type.unwrap_or(0)) != EffectType::BuffActInfoUpdate {
                continue;
            }
            let Some(info) = effect.buff_act_info.as_ref() else {
                continue;
            };
            if info.act_id != Some(BUFF_ACT_ID_RASPBERRY) {
                continue;
            }
            let Some(max_cap) = info.param.get(1).copied() else {
                continue;
            };
            if max_cap > seeded_max {
                seeded_max = max_cap;
            }
        }

        if seeded_max > 0 {
            rubuska::seed_replay_raspberry_max(&self.fight, seeded_max);
            rubuska::apply_seeded_shadow_cloak_capacity(
                &mut self.mechanics.shadow_cloak,
                seeded_max,
            );
        }
    }
}
