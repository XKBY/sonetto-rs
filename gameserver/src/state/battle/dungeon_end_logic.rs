use crate::error::AppError;
use crate::send_push;
use crate::state::{generate_dungeon_rewards, ConnectionContext};
use crate::util::push::{send_dungeon_update_push, send_end_dungeon_push, send_red_dot_push};
use database::db::game::dungeons::{
    get_user_dungeon, should_update_dungeon_record, update_dungeon_progress,
};
use database::db::game::{dungeons::save_dungeon_record, equipment::build_equip_records};
use sonettobuf::{CmdId, FightGroup, InstructionDungeonInfoPush};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle dungeon completion logic (progress updates, rewards, pushes)
/// Returns true if dungeon logic was executed, false if not a dungeon
pub async fn handle_dungeon_end(
    ctx: Arc<Mutex<ConnectionContext>>,
    pool: &SqlitePool,
    player_id: i64,
    chapter_id: i32,
    episode_id: i32,
    fight_group: &Option<FightGroup>,
    multiplication: i32,
    is_replay: bool,
    is_victory: bool, // true = victory, false = loss/abort
) -> Result<bool, AppError> {
    // Check if this is a dungeon battle
    if chapter_id <= 0 || episode_id <= 0 {
        return Ok(false);
    }
    // Trial hero battles (all hero UIDs negative) never produce a replayable record.
    let is_trial_fight = fight_group
        .as_ref()
        .map(|fg| {
            let non_zero: Vec<i64> = fg.hero_list.iter().filter(|&&u| u != 0).copied().collect();
            !non_zero.is_empty() && non_zero.iter().all(|&u| u < 0)
        })
        .unwrap_or(false);

    if is_victory && !is_replay {
        // Update player's dungeon progress
        let stars_earned = 2; // TODO: Calculate based on performance
        update_dungeon_progress(pool, player_id, chapter_id, episode_id, stars_earned).await?;

        // TODO: Get actual round count from battle state
        let record_round = 1;
        let should_save_record = if is_trial_fight {
            false
        } else {
            should_update_dungeon_record(pool, player_id, episode_id, record_round, fight_group)
            .await?
        };
        if should_save_record {
            let equips = build_equip_records(pool, player_id, fight_group).await?;

            // Resolve the actual trial hero IDs for any negative-uid slots.
            // Negative UIDs (-1, -2, ...) are slot indices that map to a battle's
            // trialHeros config field (e.g. "3121004|3122032"). We resolve those IDs
            // here (gameserver has config access) so the replay client can render the
            // correct hero models.
            let trial_ids: Vec<i32> = {
                let fg = fight_group.as_ref().map(|f| &f.hero_list[..]).unwrap_or(&[]);
                let mut ids = Vec::new();
                if fg.iter().any(|&uid| uid < 0) {
                    let game_data = config::configs::get();
                    // Look up battle_id from episode config
                    let battle_id = game_data.episode.iter()
                        .find(|e| e.id == episode_id)
                        .map(|e| e.battle_id)
                        .unwrap_or(0);
                    let trial_heros_str = game_data.battle.iter()
                        .find(|b| b.id == battle_id)
                        .map(|b| b.trial_heros.as_str())
                        .unwrap_or("");

                    // Parse the trialHeros field: "3121004|3122032" -> [3121004, 3122032]
                    let trial_hero_ids: Vec<i32> = trial_heros_str
                        .split('|')
                        .filter_map(|entry| entry.split('#').next()?.trim().parse::<i32>().ok())
                        .collect();

                    // For each negative-uid slot (-1=slot0, -2=slot1), take the corresponding id
                    let mut sorted_negative: Vec<i64> = fg.iter().filter(|&&u| u < 0).copied().collect();
                    sorted_negative.sort_by_key(|&u| -u); // -1 first, -2 second, ...
                    for slot_uid in sorted_negative {
                        let slot = ((-slot_uid) - 1) as usize;
                        if let Some(&tid) = trial_hero_ids.get(slot) {
                            ids.push(tid);
                        }
                    }
                }
                ids
            };

            save_dungeon_record(
                pool,
                player_id,
                episode_id,
                record_round,
                &fight_group.clone().unwrap_or_default(),
                equips,
                trial_ids,
            )
            .await?;
        }

        tracing::info!(
            "Dungeon completed: episode={}, round={}, record_saved={}",
            episode_id,
            record_round,
            should_save_record
        );
    }

    if is_victory {
        if !is_replay {
            // Send dungeon completion pushes for victory (normal play only).
            // Replay is purely cosmetic — it must not re-award or re-trigger story.
            send_push!(
                ctx,
                CmdId::DungeonInstructionDungeonInfoPushCmd,
                InstructionDungeonInfoPush,
                "dungeon/instruction_dungeon_info.json"
            );

            let updated_dungeon = get_user_dungeon(pool, player_id, chapter_id, episode_id).await?;

            let game_data = config::configs::get();
            let chapter_type = game_data
                .chapter
                .iter()
                .find(|c| c.id == chapter_id)
                .map(|c| c.r#type)
                .unwrap_or(6);
            // The episode.story field ("type#sceneId#storyId") references a special
            // live2D/cinematic story sequence. Passing it via extra_str tells the client
            // to play this sequence before showing the result screen.
            let story_str = game_data
                .episode
                .iter()
                .find(|e| e.id == episode_id)
                .map(|e| e.story.clone())
                .unwrap_or_default();


            send_dungeon_update_push(
                ctx.clone(),
                chapter_id,
                episode_id,
                updated_dungeon.star,
                updated_dungeon.challenge_count,
                updated_dungeon.has_record,
                chapter_type,
                2, // TODO: Calculate today's chapter completions
                2, // TODO: Calculate today's chapter attempts
            )
                .await?;

            // Generate and send rewards
            let is_first_clear = updated_dungeon.challenge_count == 1;
            let rewards = generate_dungeon_rewards(episode_id, is_first_clear, multiplication);

            let mut all_rewards = rewards.normal_bonus.clone();
            all_rewards.extend(rewards.first_bonus);
            all_rewards.extend(rewards.free_bonus);

            send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, all_rewards, is_first_clear, story_str).await?;

            send_red_dot_push(Arc::clone(&ctx), player_id, Some(vec![1027, 1047])).await?;
        } else {
            tracing::info!(
                "handle_dungeon_end: is_replay=true, skipping victory pushes/rewards for episode={}",
                episode_id
            );
            // Still send an empty EndDungeonPush so the client can exit the replay screen cleanly.
            send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, vec![], false, String::new()).await?;
        }
    } else {
        // Send empty end dungeon push for loss/abort
        send_end_dungeon_push(ctx.clone(), chapter_id, episode_id, vec![], false, String::new()).await?;
    }

    Ok(true)
}
