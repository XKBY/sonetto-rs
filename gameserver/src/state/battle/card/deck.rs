use crate::error::AppError;
use rand::{Rng, SeedableRng, rngs::StdRng, thread_rng};
use sonettobuf::{CardInfo, CardInfoPush, Fight, FightGroup};
use sqlx::SqlitePool;

use super::draw::draw_deck_guaranteed_by_uid_with_rng;
use super::pool::build_candidate_pool;

pub async fn generate_initial_deck(
    pool: &SqlitePool,
    user_id: i64,
    fight_group: &FightGroup,
    act_point: i32,
) -> Result<CardInfoPush, AppError> {
    let active_heroes: Vec<i64> = fight_group
        .hero_list
        .iter()
        .copied()
        .filter(|&u| u != 0)
        .collect();
    let candidates = build_candidate_pool(pool, user_id, &active_heroes).await?;
    let opening_hand_size = (active_heroes.len() + 4).min(9);
    let mut rng = thread_rng();

    let dealt_cards = draw_deck_guaranteed_by_uid_with_rng(
        &candidates,
        &active_heroes,
        opening_hand_size,
        &mut rng,
    );

    Ok(CardInfoPush {
        card_group: dealt_cards.clone(),
        deal_card_group: dealt_cards,
        act_point: Some(act_point),
        move_num: Some(0),
        before_cards: vec![],
        extra_move_act: Some(0),
        is_gm: Some(false),
    })
}

pub async fn generate_ai_deck(fight: &Fight, seed: u64) -> Vec<CardInfo> {
    let mut rng = StdRng::seed_from_u64(seed);
    let Some(defender) = &fight.defender else {
        return vec![];
    };

    // Collect alive attacker UIDs (player heroes) - these are the targets for enemy attacks
    let attacker_uids: Vec<i64> = fight
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .filter_map(|e| {
                    let uid = e.uid.unwrap_or(0);
                    if uid > 0 && e.current_hp.unwrap_or(0) > 0 {
                        Some(uid)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if attacker_uids.is_empty() {
        return vec![];
    }

    let mut cards = Vec::new();

    for enemy in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
        let enemy_uid = enemy.uid.unwrap_or(0);
        if enemy_uid >= 0 || enemy.current_hp.unwrap_or(0) <= 0 {
            continue;
        }
        let Some(&skill_id) = enemy.skill_group1.first() else {
            continue;
        };
        // Pick a random player hero as the target
        let target_uid = attacker_uids[rng.gen_range(0..attacker_uids.len())];
        cards.push(CardInfo {
            uid: Some(enemy_uid),
            skill_id: Some(skill_id),
            target_uid: Some(target_uid),
            card_effect: Some(0),
            temp_card: Some(false),
            enchants: vec![],
            card_type: Some(0),
            hero_id: enemy.model_id,
            status: Some(0),
            extra_info: None,
            energy: Some(0),
            extra_infos: vec![],
            area_red_or_blue: Some(0),
            heat_id: Some(0),
            music_note: None,
        });
    }
    cards
}

pub fn default_max_ap(episode_id: i32, hero_count: usize) -> i32 {
    let game_data = config::configs::get();

    let battle_id = game_data
        .episode
        .iter()
        .find(|t| t.id == episode_id)
        .map(|t| t.battle_id)
        .unwrap_or(0);

    let base_ap = game_data
        .battle
        .iter()
        .find(|t| t.id == battle_id)
        .map(|t| t.player_max)
        .unwrap_or(0);

    let hero_ap = match hero_count {
        0..=2 => 2,
        _ => 4,
    };

    base_ap.min(hero_ap)
}
