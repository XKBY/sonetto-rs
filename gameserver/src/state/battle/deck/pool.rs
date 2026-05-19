use crate::state::battle::entity::{destiny::Destiny, skill::Skill};
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

pub fn generate_deck(entries: &[(i64, i32, i32, i32, bool)]) -> Vec<CardInfo> {
    let mut deck = Vec::new();
    for &(uid, hero_id, skill1, skill2, is_trial) in entries {
        for &skill_id in &[skill1, skill2] {
            if skill_id == 0 { continue; }
            for _ in 0..8 {
                deck.push(make_card(hero_id, skill_id, uid, is_trial));
            }
        }
    }
    deck
}

pub async fn build_player_deck(
    pool: &SqlitePool,
    user_id: i64,
    hero_uids: &[i64],
) -> Result<Vec<CardInfo>, AppError> {
    let hero_db = UserHeroModel::new(user_id, pool.clone());
    let mut entries: Vec<(i64, i32, i32, i32, bool)> = Vec::new();

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

        let skill1 = group1.first().copied().unwrap_or(0);
        let skill2 = group2.first().copied().unwrap_or(0);
        entries.push((hero_uid, hero_id, skill1, skill2, hero_uid < 0));
    }

    Ok(generate_deck(&entries))
}

pub fn build_enemy_deck(monster_ids: &[i32]) -> Vec<CardInfo> {
    let game_data = configs::get();
    let entries: Vec<(i64, i32, i32, i32, bool)> = monster_ids
        .iter()
        .filter_map(|&monster_id| {
            let monster = game_data.monster.iter().find(|m| m.id == monster_id)?;
            let template = game_data.monster_skill_template.iter().find(|s| s.id == monster.skill_template)?;
            let uid = monster_id as i64;
            let skill1 = crate::state::battle::entity::skill::parse_skill_group(&template.active_skill, 1).into_iter().next().unwrap_or(0);
            let skill2 = crate::state::battle::entity::skill::parse_skill_group(&template.active_skill, 2).into_iter().next().unwrap_or(0);
            Some((uid, monster_id, skill1, skill2, false))
        })
        .collect();
    generate_deck(&entries)
}

pub fn make_card(hero_id: i32, skill_id: i32, uid: i64, is_trial: bool) -> CardInfo {
    CardInfo {
        uid: Some(uid),
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
