use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{ConnectionContext, ReplayRoundData};
use database::db::game::battle::save_round_operations;
use prost::Message;
use sonettobuf::{BeginRoundReply, BeginRoundRequest, CardInfo, CmdId};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_begin_round(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = BeginRoundRequest::decode(&req.data[..])?;

    let (is_replay, battle_id, episode_id, round_num, mut fight_data_mgr, replay_round_data) = {
        let mut conn = ctx.lock().await;
        let battle = conn
            .active_battle
            .as_mut()
            .ok_or(AppError::InvalidRequest)?;
        let mgr = battle
            .fight_data_mgr
            .take()
            .ok_or(AppError::InvalidRequest)?;
        // Pop pre-loaded replay data for this round (server-side).
        // Each entry carries both the original ops and the pre-round hand state.
        let stored = battle.replay_opers.pop_front();
        (
            battle.is_replay.unwrap_or(false),
            battle.fight_id.unwrap_or_default(),
            battle.episode_id,
            mgr.fight().cur_round.unwrap_or(1),
            mgr,
            stored,
        )
    };

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    // In normal mode: snapshot the hand BEFORE ops are applied — this is what we save.
    // We do this before process_round mutates the deck manager.
    let pre_round_hand: Vec<CardInfo> = if !is_replay {
        fight_data_mgr.managers.deck_mgr.player_hand.clone()
    } else {
        vec![]
    };

    // Choose which ops to use:
    //   Replay mode: use pre-loaded stored ops AND restore the original hand first.
    //   Normal mode: use what the client sent.
    let effective_opers = if is_replay {
        if let Some(ReplayRoundData { opers, pre_round_hand: saved_hand }) = replay_round_data {
            tracing::info!(
                "begin_round: replay mode — restoring hand ({} cards) and using {} stored oper(s)",
                saved_hand.len(),
                opers.len()
            );
            // Restore the exact hand the player had at the start of this round.
            // This makes card indices in the ops refer to the correct cards
            // regardless of any RNG divergence since the original battle.
            if !saved_hand.is_empty() {
                fight_data_mgr.managers.deck_mgr.player_hand = saved_hand;
            }
            opers
        } else {
            tracing::warn!(
                "begin_round: replay mode but no stored data for round {} — using client opers",
                round_num
            );
            request.opers.clone()
        }
    } else {
        request.opers.clone()
    };

    tracing::info!(
        "begin_round: calling process_round episode={} round={}",
        episode_id,
        round_num
    );

    let round_result = fight_data_mgr.process_round(effective_opers.clone(), None).await;

    // ALWAYS restore fight_data_mgr before propagating any error.
    {
        let mut conn = ctx.lock().await;
        if let Some(battle) = conn.active_battle.as_mut() {
            fight_data_mgr.last_round = round_result.as_ref().ok().cloned();
            battle.fight_data_mgr = Some(fight_data_mgr);
        }
    }

    let round = match round_result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("begin_round: process_round FAILED: {}", e);
            return Err(e.into());
        }
    };

    tracing::info!(
        "begin_round: process_round done steps={} is_finish={:?} cur_round={:?}",
        round.fight_step.len(),
        round.is_finish,
        round.cur_round,
    );

    let reply = BeginRoundReply { round: Some(round.clone()) };
    let encoded_len = reply.encoded_len();
    tracing::info!("begin_round: sending BeginRoundReply encoded_len={} bytes", encoded_len);

    let send_result = {
        let mut conn = ctx.lock().await;
        conn.send_reply(CmdId::BeginRoundCmd, reply, 0, req.up_tag).await
    };
    match &send_result {
        Ok(_) => tracing::info!("begin_round: send_reply OK"),
        Err(e) => tracing::error!("begin_round: send_reply FAILED: {}", e),
    }
    send_result?;

    // Save round data for future replays (normal mode only).
    if !is_replay {
        if let Err(e) = save_round_operations(
            &pool,
            player_id,
            episode_id,
            battle_id,
            round_num,
            vec![],
            effective_opers,
            pre_round_hand,
        ).await {
            tracing::warn!(
                "begin_round: save_round_operations failed (non-fatal): \
                 episode={} battle_id={} err={}",
                episode_id, battle_id, e
            );
        }
    }

    tracing::info!("begin_round: complete");
    Ok(())
}