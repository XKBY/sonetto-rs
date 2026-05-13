use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{BattleSimulator, ConnectionContext};
use database::db::game::battle::save_round_operations;
use prost::Message;
use sonettobuf::{BeginRoundReply, BeginRoundRequest, CmdId};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_begin_round(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = BeginRoundRequest::decode(&req.data[..])?;

    tracing::info!(
        "BeginRound: {} operations, auto={}",
        request.opers.len(),
        request.auto_oper.unwrap_or(false)
    );

    let (
        current_deck,
        fight_group,
        chapter_id,
        episode_id,
        is_replay,
        battle_id,
        round_num,
        multiplication,
        ai_deck,
        fight_data_mgr,
    ) = {
        let mut conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_mut()
            .ok_or(AppError::InvalidRequest)?;

        let mgr = battle
            .fight_data_mgr
            .take()
            .ok_or(AppError::InvalidRequest)?;

        (
            battle.current_deck.clone(),
            battle.fight_group.clone(),
            battle.chapter_id,
            battle.episode_id,
            battle.is_replay.unwrap_or(false),
            battle.fight_id.unwrap_or_default(),
            battle.current_round,
            battle.multiplication.unwrap_or(1),
            battle.ai_deck.clone(),
            mgr,
        )
    };

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let mut simulator = BattleSimulator::new(fight_data_mgr);

    let round_num_played = round_num;
    let round = simulator
        .process_round(request.opers.clone(), current_deck, ai_deck, None)
        .await?;
    let fight_data_mgr = simulator.into_data();
    let is_finish = round.is_finish.unwrap_or(false);
    let simulator_next_round = round
        .cur_round
        .unwrap_or(round_num_played.saturating_add(1));
    let next_round_num = simulator_next_round.max(round_num_played.saturating_add(1));
    let record_round = round.cur_round.unwrap_or(1);

    {
        let mut conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_mut()
            .ok_or(AppError::InvalidRequest)?;
        battle.fight_data_mgr = Some(fight_data_mgr);
        battle.current_round = next_round_num;
    }

    tracing::info!(
        "Round result: {} steps, {} cards, round={}, finish={}",
        round.fight_step.len(),
        round.team_a_cards1.len(),
        record_round,
        is_finish
    );

    let reply = BeginRoundReply { round: Some(round) };

    {
        let mut conn = ctx.lock().await;
        conn.send_reply(CmdId::BeginRoundCmd, reply, 0, req.up_tag)
            .await?;
    }

    if !is_replay {
        // Save operations for replay
        save_round_operations(
            &pool,
            player_id,
            episode_id,
            battle_id,
            round_num_played,
            vec![], // TODO: Extract cloth_skill_opers from request
            request.opers,
        )
        .await?;
    }

    if !is_finish {
        tracing::info!(
            "Round ongoing: episode={}, played_round={}, next_round={}",
            episode_id,
            round_num_played,
            next_round_num
        );
    }

    Ok(())
}
