use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{
    ActiveBattle, BattleContext, ConnectionContext, apply_opening_deck, build_player_deck,
    create_battle, default_max_ap, generate_initial_player_hand,
};
use config::configs;
use database::db::game::dungeons::{get_user_dungeon, update_dungeon_progress};
use prost::Message;
use sonettobuf::{CmdId, DungeonUpdatePush, StartDungeonReply, StartDungeonRequest, UserDungeon};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_start_dungeon(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = StartDungeonRequest::decode(&req.data[..])?;
    tracing::info!("Received start dungeon request {:?}", request);

    let chapter_id = request.chapter_id.unwrap_or(0);
    let episode_id = request.episode_id.unwrap_or(0);
    let use_record = request.use_record.unwrap_or(false);
    let multiplication = request.multiplication.unwrap_or(1);

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let game_data = configs::get();

    let episode_cfg = game_data
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or(AppError::InvalidRequest)?;

    if episode_cfg.battle_id == 0 {
        return handle_story_only_episode(ctx, req, chapter_id, episode_id).await;
    }

    let fight_group = request.fight_group.ok_or(AppError::InvalidRequest)?;

    let hero_count = fight_group.hero_list.iter().filter(|&&u| u != 0).count();

    let battle_id = episode_cfg.battle_id;
    let max_ap = default_max_ap(episode_id, hero_count);

    let battle_ctx = BattleContext {
        player_id,
        chapter_id,
        episode_id,
        battle_id,
        max_ap,
    };

    let mut card_push = generate_initial_player_hand(&pool, player_id, &fight_group, max_ap).await?;

    // Initial round should use raw dealt cards.
    let card_deck = card_push.deal_card_group.clone();

    let (initial_round, mut fight_data_mgr, ai_deck) =
        create_battle(&pool, battle_ctx, &fight_group, card_deck.clone()).await?;
    let all_hero_uids: Vec<i64> = fight_group
        .hero_list
        .iter()
        .chain(fight_group.sub_hero_list.iter())
        .copied()
        .filter(|&u| u != 0)
        .collect();
    let player_deck = build_player_deck(&pool, player_id, &all_hero_uids)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("build_player_deck failed at battle start: {e}");
            vec![]
        });
    // Authoritative post-start deck = pushed opening hand + opening temp/special additions.
    let mut push_round = initial_round.clone();
    push_round.team_a_cards1 = card_push.card_group.clone();
    let final_cards = apply_opening_deck(&mut push_round);
    card_push.card_group = final_cards.clone();

    // weird visual bugs if we don't split
    // ig fight steps apply damage then second object applies. but we can't do that in the intial object
    let fight_snapshot = fight_data_mgr
        .pre_fight
        .clone()
        .unwrap_or_else(|| fight_data_mgr.fight().clone()); // pre-sync fight
    // intial fight object with no passive changes

    let fight_for_battle = fight_data_mgr.fight().clone(); // post-sync fight
    // final fight object with passive changes applied

    {
        let mut conn = ctx.lock().await;
        conn.active_battle = Some(ActiveBattle {
            tower_type: None,
            tower_id: None,
            layer_id: None,
            episode_id,
            chapter_id,
            difficulty: None,
            talent_plan_id: None,
            fight: Some(fight_for_battle),
            current_round: 1,
            act_point: max_ap,
            power: 15,
            player_hand: final_cards,
            player_deck,
            fight_group: Some(fight_group.clone()),
            is_replay: Some(use_record),
            replay_episode_id: Some(episode_id),
            fight_id: Some(chrono::Utc::now().timestamp_millis()),
            multiplication: Some(multiplication),
            ai_deck,
            fight_data_mgr: Some(fight_data_mgr),
        });
    }

    /*  let updated_dungeon = get_user_dungeon(&pool, player_id, chapter_id, episode_id).await?;

    let chapter_type = game_data
        .chapter
        .iter()
        .find(|c| c.id == chapter_id)
        .map(|c| c.r#type)
        .unwrap_or(6);

    let chapter_type_nums = vec![sonettobuf::UserChapterTypeNum {
        chapter_type: Some(chapter_type),
        today_pass_num: Some(1),
        today_total_num: Some(2),
    }];

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(updated_dungeon.star),
            challenge_count: Some(updated_dungeon.challenge_count),
            has_record: Some(updated_dungeon.has_record),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums,
    };*/

    let reply = StartDungeonReply {
        fight: Some(fight_snapshot),
        round: Some(initial_round),
    };

    let mut conn = ctx.lock().await;

    // weird this doesn't belong here lol

    //conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push).await?;
    //

    tracing::warn!(
        "reply round ex_point_info[0] current_hp={:?}",
        reply
            .round
            .as_ref()
            .and_then(|r| r.ex_point_info.first())
            .and_then(|e| e.current_hp)
    );

    conn.send_reply(CmdId::StartDungeonCmd, reply, 0, req.up_tag)
        .await?;

    // This needs to be sent after the reply else client bugs out
    conn.notify(CmdId::CardInfoPushCmd, card_push).await?;

    Ok(())
}

async fn handle_story_only_episode(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
    chapter_id: i32,
    episode_id: i32,
) -> Result<(), AppError> {
    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    // Mark as completed with 1 star since it's story-only
    update_dungeon_progress(&pool, player_id, chapter_id, episode_id, 1).await?;

    // Fetch the updated record
    let updated_dungeon = get_user_dungeon(&pool, player_id, chapter_id, episode_id).await?;

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(updated_dungeon.star),
            challenge_count: Some(updated_dungeon.challenge_count),
            has_record: Some(updated_dungeon.has_record),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums: vec![],
    };

    let reply = StartDungeonReply {
        fight: None,
        round: None,
    };

    let mut conn = ctx.lock().await;

    conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push)
        .await?;

    conn.send_reply(CmdId::StartDungeonCmd, reply, 0, req.up_tag)
        .await?;

    Ok(())
}
