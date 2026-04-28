use crate::state::battle::context::RoundContext;
use crate::state::battle::manager::{
    card_mgr::FightCardMgr, fight_data_mgr::FightDataMgr, round_mgr::FightRoundMgr,
};
use crate::state::battle::round_state::set_simulated_round;
use anyhow::Result;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sonettobuf::{BeginRoundOper, CardInfo, FightRound, FightStep};

pub struct BattleSimulator {
    rng: StdRng,
    data: FightDataMgr,
    round_mgr: FightRoundMgr,
    card_mgr: FightCardMgr,
    rounds_processed: i32,
}

impl BattleSimulator {
    pub fn new(data: FightDataMgr) -> Self {
        let fight = data.get_fight();
        let seed = fight.cur_round.unwrap_or(0) as u64;

        tracing::info!(
            "Initialized battle with {} player entities, {} enemy entities",
            fight
                .attacker
                .as_ref()
                .map(|a| a.entitys.len())
                .unwrap_or(0),
            fight
                .defender
                .as_ref()
                .map(|d| d.entitys.len())
                .unwrap_or(0),
        );

        Self {
            rng: StdRng::seed_from_u64(seed),
            data,
            round_mgr: FightRoundMgr::new(),
            card_mgr: FightCardMgr::new(),
            rounds_processed: 0,
        }
    }

    pub async fn process_round(
        &mut self,
        operations: Vec<BeginRoundOper>,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
        ai_override_steps: Option<Vec<FightStep>>,
    ) -> Result<FightRound> {
        self.process_round_with_replay(
            operations,
            current_deck,
            ai_deck,
            ai_override_steps,
            None,
            None,
        )
        .await
    }

    pub async fn process_round_with_replay(
        &mut self,
        operations: Vec<BeginRoundOper>,
        current_deck: Vec<CardInfo>,
        ai_deck: Vec<CardInfo>,
        ai_override_steps: Option<Vec<FightStep>>,
        replay_selected_cards: Option<Vec<CardInfo>>,
        replay_silent_ops: Option<Vec<bool>>,
    ) -> Result<FightRound> {
        self.rounds_processed += 1;
        set_simulated_round(self.rounds_processed);
        let mut fight_ctx = self.data.ctx_with_rng(&mut self.rng);
        let round_index = fight_ctx.fight.cur_round.unwrap_or(1);
        let mut round_ctx = RoundContext::new(&mut fight_ctx, round_index);
        let round = self
            .round_mgr
            .process_round_with_replay(
                &mut self.rng,
                &mut round_ctx,
                &mut self.card_mgr,
                operations,
                current_deck,
                ai_deck,
                ai_override_steps,
                replay_selected_cards,
                replay_silent_ops,
            )
            .await?;
        Ok(round)
    }

    pub fn into_data(self) -> FightDataMgr {
        self.data
    }
}
