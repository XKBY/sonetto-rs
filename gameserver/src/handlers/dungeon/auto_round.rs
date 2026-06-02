use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::util::push::{send_dungeon_update_push, send_end_dungeon_push, send_red_dot_push};

use crate::send_push;
use crate::state::{
    ConnectionContext, generate_auto_opers, generate_dungeon_rewards,
    send_end_fight_push,
};
use database::db::game::dungeons::{
    get_user_dungeon, should_update_dungeon_record, update_dungeon_progress,
};
use database::db::game::{
    battle::save_round_operations, dungeons::save_dungeon_record, equipment::build_equip_records,
};
use prost::Message;
use sonettobuf::{AutoRoundReply, AutoRoundRequest, CmdId, InstructionDungeonInfoPush};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_auto_round(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = AutoRoundRequest::decode(&req.data[..])?;

    tracing::info!(
        "AutoRound request: client_opers: {:?}, client_opers_len={}, to_id={}",
        request.opers,
        request.opers.len(),
        request.to_id.unwrap_or(0)
    );

    let (
        fight_group,
        chapter_id,
        episode_id,
        is_replay,
        battle_id,
        round_num,
        multiplication,
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
            battle.fight_group.clone(),
            battle.chapter_id,
            battle.episode_id,
            battle.is_replay.unwrap_or(false),
            battle.fight_id.unwrap_or_default(),
            mgr.fight().cur_round.unwrap_or(1),
            battle.multiplication.unwrap_or(1),
            mgr,
        )
    };

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (conn.player_id.ok_or(AppError::NotLoggedIn)?, conn.state.db.clone())
    };

    let pre_round_hand = fight_data_mgr.managers.deck_mgr.player_hand.clone();

    let auto_opers = {
        let hand = fight_data_mgr.managers.deck_mgr.player_hand.clone();
        generate_auto_opers(&hand)
    };

    tracing::info!("AutoRound: processing round for episode={} battle_id={}", episode_id, battle_id);

    let mut fight_data_mgr = fight_data_mgr;
    let mut round = fight_data_mgr.process_round(auto_opers.clone(), None).await?;
    round.is_finish = Some(true);

    {
        let mut conn = ctx.lock().await;
        let battle = conn.active_battle.as_mut().ok_or(AppError::InvalidRequest)?;
        battle.fight_data_mgr = Some(fight_data_mgr);
    }

    let record_round = round.cur_round.unwrap_or(1);

    tracing::info!(
        "AutoRound result: steps={}, cards={}, round={}, finished={}",
        round.fight_step.len(),
        round.team_a_cards1.len(),
        record_round,
        round.is_finish.unwrap_or(false)
    );

    let reply = AutoRoundReply {
        opers: auto_opers.clone(),
        to_id: request.to_id.or(Some(1)),
    };

    {
        let mut conn = ctx.lock().await;
        conn.send_reply(CmdId::AutoRoundCmd, reply, 0, req.up_tag)
            .await?;
    }
    tracing::info!("AutoRound: reply sent");

    if !is_replay {
        tracing::info!("AutoRound: saving round operations player={} episode={} battle_id={} round={}", player_id, episode_id, battle_id, round_num);
        if let Err(e) = save_round_operations(
            &pool,
            player_id,
            episode_id,
            battle_id,
            round_num,
            vec![],
            auto_opers.clone(),
            pre_round_hand,
        ).await {
            tracing::warn!("AutoRound: save_round_operations failed (non-fatal): {}", e);
        }

        tracing::info!("AutoRound: updating dungeon progress");
        if let Err(e) = update_dungeon_progress(&pool, player_id, chapter_id, episode_id, 2).await {
            tracing::warn!("AutoRound: update_dungeon_progress failed (non-fatal): {}", e);
        }

        tracing::info!("AutoRound: checking dungeon record");
        let should_save_record = should_update_dungeon_record(
            &pool, player_id, episode_id, record_round, &fight_group
        ).await.unwrap_or(false);

        if should_save_record {
            tracing::info!("AutoRound: saving dungeon record");
            // Build equip records only for real player heroes (non-negative UIDs)
            let real_fight_group = fight_group.as_ref().map(|fg| {
                let mut fg = fg.clone();
                fg.hero_list.retain(|uid| *uid > 0);
                fg
            });
            let equips = if let Some(ref fg) = real_fight_group {
                build_equip_records(&pool, player_id, &Some(fg.clone())).await.unwrap_or_default()
            } else {
                vec![]
            };
            if let Err(e) = save_dungeon_record(
                &pool,
                player_id,
                episode_id,
                record_round,
                &fight_group.clone().unwrap_or_default(),
                equips,
                vec![], // auto-round battles never use trial heroes
            ).await {
                tracing::warn!("AutoRound: save_dungeon_record failed (non-fatal): {}", e);
            }
        }

        tracing::info!(
            "Auto battle completed: episode={}, round={}, record_saved={}",
            episode_id, record_round, should_save_record
        );
    }

    tracing::info!("AutoRound: sending end_fight_push");
    send_end_fight_push(
        ctx.clone(),
        battle_id,
        1,
        fight_group.clone().unwrap_or_default(),
        vec![],
        vec![],
        !is_replay,
    ).await?;

    send_push!(
        ctx,
        CmdId::DungeonInstructionDungeonInfoPushCmd,
        InstructionDungeonInfoPush,
        "dungeon/instruction_dungeon_info.json"
    );

    tracing::info!("AutoRound: getting updated dungeon");
    let updated_dungeon = get_user_dungeon(&pool, player_id, chapter_id, episode_id).await?;

    let game_data = config::configs::get();
    let chapter_type = game_data
        .chapter
        .iter()
        .find(|c| c.id == chapter_id)
        .map(|c| c.r#type)
        .unwrap_or(6);

    tracing::info!("AutoRound: sending dungeon_update_push");
    send_dungeon_update_push(
        ctx.clone(),
        chapter_id,
        episode_id,
        updated_dungeon.star,
        updated_dungeon.challenge_count,
        updated_dungeon.has_record,
        chapter_type,
        2,
        2,
    ).await?;

    let is_first_clear = updated_dungeon.challenge_count == 1;
    let rewards = generate_dungeon_rewards(episode_id, is_first_clear, multiplication);

    let mut all_rewards = rewards.normal_bonus.clone();
    all_rewards.extend(rewards.first_bonus);
    all_rewards.extend(rewards.free_bonus);

    tracing::info!("AutoRound: sending end_dungeon_push with {} rewards", all_rewards.len());
    send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, all_rewards).await?;
    send_red_dot_push(ctx.clone(), player_id, Some(vec![1027, 1047])).await?;

    tracing::info!("AutoRound: complete");
    Ok(())
}