use crate::error::AppError;
use rand::{Rng, SeedableRng, rngs::StdRng, thread_rng};
use sonettobuf::{CardInfo, CardInfoPush, Fight, FightGroup};
use sqlx::SqlitePool;
use std::collections::HashSet;

use super::draw::draw_deck_guaranteed_by_uid_with_rng;
use super::pool::{build_enemy_deck, build_player_deck};
use super::upgrade::apply_card_upgrades;

pub(crate) fn card_limit(alive_count: usize, has_support: bool) -> usize {
    match alive_count {
        1 => 4,
        2 => 5,
        3 => if has_support { 7 } else { 6 },
        4 => 8,
        _ => (alive_count + 4).min(9),
    }
}

pub(crate) fn purge_dead_entity_cards(
    deck: &mut Vec<CardInfo>,
    alive_uids: &HashSet<i64>,
) {
    deck.retain(|c| {
        let uid = c.uid.unwrap_or(0);
        uid == 0 || c.temp_card.unwrap_or(false) || alive_uids.contains(&uid)
    });
}

pub async fn generate_initial_hand(
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
    let candidates = build_player_deck(pool, user_id, &active_heroes).await?;

    let hero_count = active_heroes.len();
    let has_support = fight_group.sub_hero_list.len() > 0;
    let opening_hand_size = card_limit(hero_count, has_support);

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
    let mut rng: StdRng = StdRng::seed_from_u64(seed);
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

pub fn generate_initial_enemy_hand(monster_ids: &[i32]) -> Vec<CardInfo> {
    let candidates = build_enemy_deck(monster_ids);
    let required_uids: Vec<i64> = monster_ids.iter().map(|&id| id as i64).collect();

    let opening_hand_size = card_limit(monster_ids.len(), false);

    let mut rng: rand::prelude::ThreadRng = thread_rng();
    draw_deck_guaranteed_by_uid_with_rng(&candidates, &required_uids, opening_hand_size, &mut rng)
}

pub(crate) fn refill_hand(
    rng: &mut impl Rng,
    hand: &mut Vec<CardInfo>,
    deck: &mut Vec<CardInfo>,
    ex_deck: &mut Vec<CardInfo>,
    alive_uids: &HashSet<i64>,
    extra: usize,
    fight: &Fight,
) -> Vec<CardInfo> {
    let has_support = fight.attacker.as_ref().map_or(false, |a| {
        a.sub_entitys.iter().any(|e| e.uid.unwrap_or(0) > 0)
    });
    let target_size = card_limit(alive_uids.len(), has_support) + extra;
    if deck.is_empty() && ex_deck.is_empty() {
        tracing::warn!("refill_hand: both decks empty, cannot refill");
        return vec![];
    }
    tracing::info!(target: "refill_hand", before = ?hand.iter().map(|c| c.skill_id.unwrap_or(0)).collect::<Vec<_>>(), target_size, ex_deck_len = ex_deck.len());
    let mut pulled_raw: Vec<CardInfo> = Vec::new();
    // Drain EX cards first (preferential)
    while hand.len() < target_size && !ex_deck.is_empty() {
        let card = ex_deck.remove(0);
        pulled_raw.push(card.clone());
        hand.push(card);
        apply_card_upgrades(hand, fight);
    }
    // Fill remaining slots from deck
    while hand.len() < target_size && !deck.is_empty() {
        let idx = rng.gen_range(0..deck.len());
        let raw = deck.remove(idx);
        pulled_raw.push(raw.clone());
        hand.push(raw);
        apply_card_upgrades(hand, fight);
    }
    tracing::info!(target: "refill_hand", after = ?hand.iter().map(|c| c.skill_id.unwrap_or(0)).collect::<Vec<_>>());
    pulled_raw
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
