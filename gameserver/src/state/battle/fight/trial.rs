use super::super::entity::skill::parse_skill_group;
use anyhow::Result;
use config::{configs, hero_trial::HeroTrial};
use once_cell::sync::Lazy;
use sonettobuf::{EquipRecord, FightEntityInfo, HeroAttribute};
use std::collections::HashMap;

#[allow(dead_code)]
static TRIAL_UID_MAP: Lazy<HashMap<i64, i32>> = Lazy::new(|| {
    let game_data = config::configs::get();
    let mut map = HashMap::new();

    for (index, trial) in game_data.hero_trial.iter().enumerate() {
        let uid = -((index + 1) as i64);
        map.insert(uid, trial.id);
    }

    map
});

#[allow(dead_code)]
pub struct Trial;

#[allow(dead_code)]
impl Trial {
    pub fn get(hero_uid: i64, position: i32, team_type: i32) -> Result<FightEntityInfo> {
        let game_data = configs::get();

        let trial_id = TRIAL_UID_MAP
            .get(&hero_uid)
            .ok_or_else(|| anyhow::anyhow!("Unknown trial hero UID: {}", hero_uid))?;

        let trial_data = game_data
            .hero_trial
            .get(*trial_id)
            .ok_or_else(|| anyhow::anyhow!("Trial data not found for ID {}", trial_id))?;

        let hero_config = game_data
            .character
            .iter()
            .find(|h| h.id == trial_data.hero_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Hero config not found for hero_id {}", trial_data.hero_id)
            })?;

        let (hp, attack, defense, mdefense, technic) = Self::get_stats(trial_data)?;

        let skill_group1 = parse_skill_group(&hero_config.skill, 1);
        let skill_group2 = parse_skill_group(&hero_config.skill, 2);

        let attr = HeroAttribute {
            hp: Some(hp),
            attack: Some(attack),
            defense: Some(defense),
            mdefense: Some(mdefense),
            technic: Some(technic),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        };

        Ok(FightEntityInfo {
            uid: Some(hero_uid),
            model_id: Some(trial_data.hero_id),
            skin: Some(trial_data.skin),
            position: Some(position),
            entity_type: Some(1),
            user_id: Some(0),
            ex_point: Some(0),
            level: Some(trial_data.level),
            current_hp: Some(hp),
            attr: Some(attr),
            base_attr: Some(attr),
            buffs: vec![],
            skill_group1,
            skill_group2,
            passive_skill: vec![],
            ex_skill: Some(hero_config.ex_skill),
            shield_value: Some(0),
            no_effect_buffs: vec![],
            expoint_max_add: Some(0),
            buff_harm_statistic: Some(0),
            equip_uid: Some(0),
            trial_equip: Some(EquipRecord {
                equip_uid: Some(0),
                equip_id: Some(trial_data.equip_id),
                equip_lv: Some(trial_data.equip_lv),
                refine_lv: Some(trial_data.equip_refine),
            }),
            ex_skill_level: Some(trial_data.ex_skill_lv),
            power_infos: vec![],
            act104_equip_uids: vec![],
            trial_act104_equips: vec![],
            summoned_list: vec![],
            ex_skill_point_change: Some(0),
            team_type: Some(team_type),
            enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
                uid: Some(hero_uid),
                can_upgrade_ids: vec![],
                upgraded_options: vec![],
            }),
            trial_id: Some(trial_data.id),
            career: Some(hero_config.career),
            status: Some(0),
            guard: Some(-1),
            sub_cd: Some(0),
            ex_point_type: Some(0),
            equips: vec![],
            destiny_stone: Some(0),
            destiny_rank: Some(0),
            custom_unit_id: Some(0),
        })
    }

    fn get_stats(trial_data: &HeroTrial) -> Result<(i32, i32, i32, i32, i32)> {
        let game_data = configs::get();

        let level_data = game_data
            .character_level
            .iter()
            .find(|c| c.hero_id == trial_data.hero_id && c.level == trial_data.level)
            .or_else(|| {
                tracing::warn!(
                    "Level {} not found for hero {}, falling back to level 1",
                    trial_data.level,
                    trial_data.hero_id
                );
                game_data
                    .character_level
                    .iter()
                    .find(|c| c.hero_id == trial_data.hero_id && c.level == 1)
            })
            .ok_or_else(|| anyhow::anyhow!("No level data for hero_id {}", trial_data.hero_id))?;

        Ok((
            level_data.hp,
            level_data.atk,
            level_data.def,
            level_data.mdef,
            level_data.technic,
        ))
    }
}
