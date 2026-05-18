use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::{
    ActiveBattle, BattleContext, ConnectionContext, apply_opening_deck, build_enemy_deck,
    build_player_deck, create_battle, default_max_ap, generate_initial_enemy_hand,
    generate_initial_hand,
};
use config::configs;
use prost::Message;
use sonettobuf::{
    CmdId, DungeonUpdatePush, StartDungeonReply, StartTowerBattleReply, StartTowerBattleRequest,
    UserDungeon,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_start_tower_battle(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = StartTowerBattleRequest::decode(&req.data[..])?;

    let start_req = request
        .start_dungeon_request
        .ok_or(AppError::InvalidRequest)?;
    let fight_group = start_req.fight_group.ok_or(AppError::InvalidRequest)?;

    let dungeon_type = request.r#type.ok_or(AppError::InvalidRequest)?;
    let tower_id = request.tower_id.ok_or(AppError::InvalidRequest)?;
    let layer_id = request.layer_id.ok_or(AppError::InvalidRequest)?;
    let difficulty = request.difficulty.ok_or(AppError::InvalidRequest)?;
    let talent_plan_id = request.talent_plan_id.unwrap_or(0);

    let chapter_id = start_req.chapter_id.unwrap_or(0);
    let episode_id = start_req.episode_id.unwrap_or(0);

    tracing::info!(
        "Start tower battle: type={}, tower={}, layer={}, episode={}, diff={}, talent={}",
        dungeon_type,
        tower_id,
        layer_id,
        episode_id,
        difficulty,
        talent_plan_id
    );

    tracing::info!(
        "Fight group: heroes={:?}, cloth={}, assist_boss={}",
        fight_group.hero_list,
        fight_group.cloth_id.unwrap_or(1),
        fight_group.assist_boss_id.unwrap_or(0)
    );

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (
            conn.player_id.ok_or(AppError::NotLoggedIn)?,
            conn.state.db.clone(),
        )
    };

    let hero_count = fight_group.hero_list.iter().filter(|&&u| u != 0).count();

    let game_data = configs::get();
    let battle_id = game_data
        .episode
        .iter()
        .find(|e| e.id == episode_id)
        .ok_or(AppError::InvalidRequest)?
        .battle_id;

    let max_ap = default_max_ap(episode_id, hero_count);

    let battle_ctx = BattleContext {
        player_id,
        chapter_id,
        episode_id,
        battle_id,
        max_ap,
    };

    let mut card_push = generate_initial_hand(&pool, player_id, &fight_group, max_ap).await?;

    // Initial round should use raw dealt cards.
    let card_deck = card_push.deal_card_group.clone();

    let (initial_round, mut fight_data_mgr, _) =
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

    let fight_snapshot = fight_data_mgr
        .pre_fight
        .clone()
        .unwrap_or_else(|| fight_data_mgr.fight().clone()); // pre-sync fight
    // intial fight object with no passive changes

    let fight_for_battle = fight_data_mgr.fight().clone(); // post-sync fight
    // final fight object with passive changes applied

    let monster_ids: Vec<i32> = fight_for_battle
        .defender
        .as_ref()
        .map(|d| {
            d.entitys
                .iter()
                .chain(d.sub_entitys.iter())
                .filter_map(|e| e.model_id)
                .collect()
        })
        .unwrap_or_default();
    let enemy_deck = build_enemy_deck(&monster_ids);
    let enemy_hand = generate_initial_enemy_hand(&monster_ids);

    {
        let mut conn = ctx.lock().await;
        conn.active_battle = Some(ActiveBattle {
            tower_type: Some(dungeon_type),
            tower_id: Some(tower_id),
            layer_id: Some(layer_id),
            episode_id,
            chapter_id,
            difficulty: Some(difficulty),
            talent_plan_id: Some(talent_plan_id),
            fight: Some(fight_for_battle),
            current_round: 1,
            act_point: max_ap,
            power: 15,
            player_hand: final_cards,
            player_deck,
            player_ex_deck: vec![],
            enemy_hand,
            enemy_deck,
            enemy_ex_deck: vec![],
            fight_group: Some(fight_group.clone()),
            is_replay: None,
            replay_episode_id: None,
            fight_id: Some(chrono::Utc::now().timestamp_millis()),
            multiplication: None,
            fight_data_mgr: Some(fight_data_mgr),
        });
    }

    let start_reply = StartTowerBattleReply {
        start_dungeon_reply: Some(StartDungeonReply {
            fight: Some(fight_snapshot),
            round: Some(initial_round),
        }),
        r#type: Some(dungeon_type),
        tower_id: Some(tower_id),
        layer_id: Some(layer_id),
        difficulty: Some(difficulty),
        talent_plan_id: Some(talent_plan_id),
    };

    let dungeon_push = DungeonUpdatePush {
        dungeon_info: Some(UserDungeon {
            chapter_id: Some(chapter_id),
            episode_id: Some(episode_id),
            star: Some(0),
            challenge_count: Some(0),
            has_record: Some(false),
            left_return_all_num: Some(1),
            today_pass_num: Some(0),
            today_total_num: Some(0),
        }),
        chapter_type_nums: vec![],
    };

    let mut conn = ctx.lock().await;

    conn.notify(CmdId::DungeonUpdatePushCmd, dungeon_push)
        .await?;

    conn.send_reply(CmdId::StartTowerBattleCmd, start_reply, 0, req.up_tag)
        .await?;

    {
        let mut conn = ctx.lock().await;
        conn.notify(CmdId::CardInfoPushCmd, card_push).await?;
    }

    Ok(())
}
