use crate::state::battle::context::RoundContext;
use crate::state::battle::manager::{
    fight_data_mgr::FightDataMgr, round_mgr::FightRoundMgr,
};
use crate::state::battle::round_state::set_simulated_round;
use crate::state::battle::skill::SkillExecutor;
use anyhow::Result;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sonettobuf::{BeginRoundOper, CardInfo, Fight, FightRound, FightStep};

pub struct BattleSimulator {
    rng: StdRng,
    data: FightDataMgr,
    round_mgr: FightRoundMgr,
    pub(crate) skill_executor: SkillExecutor,
    rounds_processed: i32,
}

impl BattleSimulator {
    pub fn new(data: FightDataMgr) -> Self {
        let fight = data.get_fight();
        let seed = fight.cur_round.unwrap_or(0) as u64;
        tracing::info!(
            "Initialized battle with {} player entities, {} enemy entities",
            fight.attacker.as_ref().map(|a| a.entitys.len()).unwrap_or(0),
            fight.defender.as_ref().map(|d| d.entitys.len()).unwrap_or(0),
        );
        Self {
            rng: StdRng::seed_from_u64(seed),
            data,
            round_mgr: FightRoundMgr::new(),
            skill_executor: SkillExecutor::new(),
            rounds_processed: 0,
        }
    }

    pub async fn process_round(
        &mut self,
        operations: Vec<BeginRoundOper>,
        ai_override_steps: Option<Vec<FightStep>>,
    ) -> Result<FightRound> {
        self.process_round_with_replay(operations, ai_override_steps, None, None, None).await
    }

    pub async fn process_round_with_replay(
        &mut self,
        operations: Vec<BeginRoundOper>,
        ai_override_steps: Option<Vec<FightStep>>,
        replay_selected_cards: Option<Vec<CardInfo>>,
        replay_silent_ops: Option<Vec<bool>>,
        replay_wave_snapshots: Option<Vec<Fight>>,
    ) -> Result<FightRound> {
        self.rounds_processed += 1;
        set_simulated_round(self.rounds_processed);
        let result = {
            let mut fight_ctx = self.data.ctx_with_rng(&mut self.rng);
            let round_index = fight_ctx.fight.cur_round.unwrap_or(1);
            let mut round_ctx = RoundContext::new(&mut fight_ctx, round_index);
            self.round_mgr
                .process_round_with_replay(
                    &mut self.rng,
                    &mut round_ctx,
                    &mut self.skill_executor,
                    operations,
                    ai_override_steps,
                    replay_selected_cards,
                    replay_silent_ops,
                    replay_wave_snapshots.as_deref(),
                )
                .await
        };
        result
    }

    pub fn check_battle_result(&self) -> i32 {
        let fight = self.data.get_fight();
        let enemies_alive = fight.defender.as_ref()
            .map(|d| d.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);
        let heroes_alive = fight.attacker.as_ref()
            .map(|a| a.entitys.iter().any(|e| e.current_hp.unwrap_or(0) > 0))
            .unwrap_or(false);
        if !heroes_alive { 0 } else if !enemies_alive { 1 } else { 1 }
    }

    pub fn into_parts(self) -> FightDataMgr {
        self.data
    }
}
