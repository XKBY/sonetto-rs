use super::super::entity::{builder::EntityBuilder, skill::parse_skill_group};
use super::team::Team;
use anyhow::Result;
use config::configs;
use sonettobuf::{EquipRecord, FightEntityInfo, FightTeam, HeroAttribute, PowerInfo};

pub struct Defender;

pub struct DefenderSetup {
    pub max_round: i32,
    pub team: FightTeam,
}

impl Defender {
    pub async fn get(episode_id: i32) -> Result<DefenderSetup> {
        let game_data = configs::get();

        let episode = game_data
            .episode
            .iter()
            .find(|e| e.id == episode_id)
            .ok_or_else(|| anyhow::anyhow!("Episode {} not found", episode_id))?;

        let battle = game_data
            .battle
            .iter()
            .find(|b| b.id == episode.battle_id)
            .ok_or_else(|| anyhow::anyhow!("Battle {} not found", episode.battle_id))?;

        let max_round = battle.max_round;
        let (entitys, sub_entitys) = Self::build_initial_wave_entities(episode.battle_id, 2)?;

        let player_entity = EntityBuilder::player(0, 2);
        let team = Team::build(
            entitys,
            sub_entitys,
            player_entity,
            Some(0),
            Some(0),
            vec![],
        );

        Ok(DefenderSetup { max_round, team })
    }

    pub(crate) fn build_wave_entities(
        battle_id: i32,
        wave: i32,
        team_type: i32,
    ) -> Result<Vec<FightEntityInfo>> {
        let game_data = configs::get();
        let battle = game_data
            .battle
            .iter()
            .find(|b| b.id == battle_id)
            .ok_or_else(|| anyhow::anyhow!("Battle {} not found", battle_id))?;

        let group_id: i32 = battle
            .monster_group_ids
            .split('#')
            .nth(wave.saturating_sub(1) as usize)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                anyhow::anyhow!("No monster group for battle {} wave {}", battle_id, wave)
            })?;

        let group = game_data
            .monster_group
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| anyhow::anyhow!("MonsterGroup {} not found", group_id))?;

        let monster_max = battle.monster_max.max(0) as usize;
        let monster_ids: Vec<i32> = group
            .monster
            .split('#')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();
        let spawn_count = if monster_max == 0 {
            monster_ids.len()
        } else {
            monster_ids.len().min(monster_max)
        };

        tracing::debug!(
            "Defender wave spawn: battle={} wave={} group={} total={} spawn={}",
            battle_id,
            wave,
            group_id,
            monster_ids.len(),
            spawn_count
        );

        monster_ids
            .into_iter()
            .take(spawn_count)
            .enumerate()
            .map(|(idx, monster_id)| {
                let position = (idx + 1) as i32;
                let uid = -((2 * (wave as i64 - 1)) + position as i64);
                Self::build_enemy_with_uid(monster_id, uid, position, team_type)
            })
            .collect()
    }

    fn build_initial_wave_entities(
        battle_id: i32,
        team_type: i32,
    ) -> Result<(Vec<FightEntityInfo>, Vec<FightEntityInfo>)> {
        let game_data = configs::get();
        let battle = game_data
            .battle
            .iter()
            .find(|b| b.id == battle_id)
            .ok_or_else(|| anyhow::anyhow!("Battle {} not found", battle_id))?;
        let monster_max = battle.monster_max as usize;

        let group_id: i32 = battle
            .monster_group_ids
            .split('#')
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("No monster group in battle {}", battle_id))?;

        let group = game_data
            .monster_group
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| anyhow::anyhow!("MonsterGroup {} not found", group_id))?;

        let monster_ids: Vec<i32> = group
            .monster
            .split('#')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();

        let initial: Vec<i32> = monster_ids.iter().copied().take(monster_max).collect();
        let queued: Vec<i32> = monster_ids.iter().copied().skip(monster_max).collect();

        tracing::debug!(
            "Defender: group={} total={} initial={} queued={}",
            group_id,
            monster_ids.len(),
            initial.len(),
            queued.len()
        );

        let mut entitys = Vec::new();
        for (idx, monster_id) in initial.iter().enumerate() {
            entitys.push(Self::build_enemy(
                *monster_id,
                idx,
                (idx + 1) as i32,
                team_type,
            )?);
        }

        let mut sub_entitys = Vec::new();
        for (i, monster_id) in queued.iter().enumerate() {
            let idx = initial.len() + i;
            let position = -((i + 1) as i32);
            sub_entitys.push(Self::build_enemy(*monster_id, idx, position, team_type)?);
        }

        Ok((entitys, sub_entitys))
    }

    fn build_enemy(
        monster_id: i32,
        idx: usize,
        position: i32,
        team_type: i32,
    ) -> Result<FightEntityInfo> {
        let uid = -((idx + 1) as i64);
        Self::build_enemy_with_uid(monster_id, uid, position, team_type)
    }

    fn build_enemy_with_uid(
        monster_id: i32,
        uid: i64,
        position: i32,
        team_type: i32,
    ) -> Result<FightEntityInfo> {
        let game_data = configs::get();

        let monster = game_data
            .monster
            .iter()
            .find(|m| m.id == monster_id)
            .ok_or_else(|| anyhow::anyhow!("Monster {} not found", monster_id))?;

        let template = game_data
            .monster_template
            .iter()
            .find(|t| t.template == monster.id)
            .ok_or_else(|| anyhow::anyhow!("Monster template {} not found", monster.id))?;

        let skill_template = game_data
            .monster_skill_template
            .iter()
            .find(|s| s.id == monster.skill_template)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Monster skill template {} not found",
                    monster.skill_template
                )
            })?;

        let level = if monster.level_true != 0 {
            monster.level_true
        } else {
            monster.level
        };

        let hp = template.life + (template.life_grow * level);
        let attack = template.attack + (template.attack_grow * level);
        let defense = template.defense + (template.defense_grow * level);
        let mdefense = template.mdefense + (template.mdefense_grow * level);
        let technic = template.technic + (template.technic_grow * level);

        let skill_group1 = parse_skill_group(&skill_template.active_skill, 1);
        let skill_group2 = parse_skill_group(&skill_template.active_skill, 2);

        let base_passives: Vec<i32> = skill_template
            .passive_skill
            .split('#')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();

        let ex_passives: Vec<i32> = monster
            .passive_skills_ex
            .split('#')
            .filter_map(|s| s.parse::<i32>().ok())
            .collect();

        // battle rule skills get injected between base and ex
        let passive_skill: Vec<i32> = base_passives.into_iter().chain(ex_passives).collect();

        let ex_skill = skill_template
            .unique_skill
            .split('#')
            .next()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);

        let attr = HeroAttribute {
            hp: Some(hp),
            attack: Some(attack),
            defense: Some(defense),
            mdefense: Some(mdefense),
            technic: Some(technic),
            multi_hp_idx: Some(0),
            multi_hp_num: Some(0),
        };

        let power_infos = if !skill_template.power_max.is_empty() {
            let parts: Vec<&str> = skill_template.power_max.split('#').collect();
            let power_id: i32 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(1);
            let max: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            if max > 0 {
                vec![PowerInfo {
                    power_id: Some(power_id),
                    num: Some(0),
                    max: Some(max),
                }]
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        Ok(FightEntityInfo {
            uid: Some(uid),
            model_id: Some(monster.id),
            skin: Some(monster.skin_id),
            position: Some(position),
            entity_type: Some(2),
            user_id: Some(0),
            ex_point: Some(0),
            level: Some(level),
            current_hp: Some(hp),
            attr: Some(attr),
            base_attr: Some(attr),
            buffs: vec![],
            skill_group1,
            skill_group2,
            passive_skill,
            ex_skill: Some(ex_skill),
            shield_value: Some(0),
            no_effect_buffs: vec![],
            expoint_max_add: Some(0),
            buff_harm_statistic: Some(0),
            equip_uid: Some(0),
            trial_equip: Some(EquipRecord {
                equip_uid: Some(0),
                equip_id: Some(0),
                equip_lv: Some(0),
                refine_lv: Some(0),
            }),
            ex_skill_level: Some(0),
            power_infos,
            act104_equip_uids: vec![],
            trial_act104_equips: vec![],
            summoned_list: vec![],
            ex_skill_point_change: Some(0),
            team_type: Some(team_type),
            enhance_info_box: Some(sonettobuf::EnhanceInfoBox {
                uid: Some(uid),
                can_upgrade_ids: vec![],
                upgraded_options: vec![],
            }),
            trial_id: Some(0),
            career: Some(skill_template.career),
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
}
