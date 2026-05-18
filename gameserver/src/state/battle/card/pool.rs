use super::super::entity::{destiny::Destiny, skill::Skill};
use crate::error::AppError;
use config::configs;
use database::models::game::heros::{HeroModel, UserHeroModel};
use once_cell::sync::Lazy;
use sonettobuf::CardInfo;
use sqlx::SqlitePool;
use std::collections::HashMap;

static TRIAL_UID_MAP: Lazy<HashMap<i64, i32>> = Lazy::new(|| {
    let game_data = configs::get();
    game_data
        .hero_trial
        .iter()
        .enumerate()
        .map(|(index, trial)| (-((index + 1) as i64), trial.hero_id))
        .collect()
});

pub async fn build_player_deck(
    pool: &SqlitePool,
    user_id: i64,
    hero_uids: &[i64],
) -> Result<Vec<CardInfo>, AppError> {
    let mut cards: Vec<CardInfo> = Vec::new();
    let hero_db = UserHeroModel::new(user_id, pool.clone());

    for &hero_uid in hero_uids {
        if hero_uid == 0 {
            continue;
        }

        let (hero_id, ex_level, destiny_map) = if hero_uid < 0 {
            let hero_id = *TRIAL_UID_MAP.get(&hero_uid).ok_or_else(|| {
                tracing::error!("Unknown trial hero UID: {}", hero_uid);
                AppError::InvalidRequest
            })?;
            (hero_id, 1, None)
        } else {
            let hero = hero_db.get_uid(hero_uid as i32).await?;
            let destiny = Destiny::get(hero.record.destiny_stone, hero.record.destiny_rank);
            (hero.record.hero_id, hero.record.ex_skill_level, destiny)
        };

        let destiny_ref = destiny_map.as_ref();
        let (group1, group2) = Skill::get_skill_groups_with_destiny(hero_id, ex_level, destiny_ref);

        if let Some(skill_id) = group1.first().copied() {
            for _ in 0..8 {
                cards.push(make_card(hero_id, skill_id, hero_uid, hero_uid < 0));
            }
        }
        if let Some(skill_id) = group2.first().copied() {
            for _ in 0..8 {
                cards.push(make_card(hero_id, skill_id, hero_uid, hero_uid < 0));
            }
        }
    }

    Ok(cards)
}

pub fn build_ai_pool(monster_ids: &[i32]) -> Vec<CardInfo> {
    let game_data = configs::get();
    let mut cards = Vec::new();

    for &monster_id in monster_ids {
        let Some(monster) = game_data.monster.iter().find(|m| m.id == monster_id) else {
            tracing::warn!("Unknown monster ID: {}", monster_id);
            continue;
        };
        let Some(skill_template) = game_data
            .monster_skill_template
            .iter()
            .find(|s| s.id == monster.skill_template)
        else {
            tracing::warn!("No skill template for monster {}", monster_id);
            continue;
        };

        let uid = monster_id as i64;
        for group in 1..=2 {
            if let Some(skill_id) =
                super::super::entity::skill::parse_skill_group(&skill_template.active_skill, group)
                    .into_iter()
                    .next()
            {
                cards.push(make_card(monster_id, skill_id, uid, false));
            }
        }
    }

    cards
}

pub(crate) fn make_card(hero_id: i32, skill_id: i32, hero_uid: i64, is_trial: bool) -> CardInfo {
    CardInfo {
        uid: Some(hero_uid),
        hero_id: Some(hero_id),
        skill_id: Some(skill_id),
        card_type: Some(0),
        status: Some(0),
        temp_card: Some(is_trial),
        enchants: vec![],
        target_uid: Some(0),
        energy: Some(0),
        extra_infos: vec![],
        area_red_or_blue: Some(0),
        heat_id: Some(0),
        card_effect: None,
        extra_info: None,
        music_note: None,
    }
}
