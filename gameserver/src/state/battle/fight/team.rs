use super::super::types::buff::BuffLayerType;
use sonettobuf::{CardHeatInfo, FightEntityInfo, FightTeam, PlayerSkillInfo};

pub struct Team;

impl Team {
    pub fn build(
        entitys: Vec<FightEntityInfo>,
        sub_entitys: Vec<FightEntityInfo>,
        player_entity: FightEntityInfo,
        power: Option<i32>,
        cloth_id: Option<i32>,
        skill_infos: Vec<PlayerSkillInfo>,
    ) -> FightTeam {
        FightTeam {
            entitys,
            sub_entitys,
            power,
            cloth_id,
            skill_infos,
            sp_entitys: vec![],
            indicators: vec![],
            ex_team_str: Some(String::new()),
            assist_boss: None,
            assist_boss_info: None,
            emitter: None,
            emitter_info: None,
            player_entity: Some(player_entity),
            player_finisher_info: None,
            energy: Some(0),
            card_heat: Some(CardHeatInfo { values: vec![] }),
            card_deck_size: Some(0),
            blood_pool: None,
            vorpalith: None,
            item_skill_group: None,
            sp_fight_entities: vec![],
            heat_scale: None,
            music_info: None,
        }
    }

    pub fn get_player_skills(cloth_id: Option<i32>) -> Vec<PlayerSkillInfo> {
        use config::configs;

        let game_data = configs::get();
        let cloth_id = cloth_id.unwrap_or(1);

        let Some(cloth) = game_data
            .cloth_level
            .iter()
            .find(|c| c.id == cloth_id && c.level == 1)
        else {
            return vec![];
        };

        let mut skills = Vec::new();

        if cloth.skill1 != 0 {
            skills.push(PlayerSkillInfo {
                skill_id: Some(cloth.skill1),
                cd: Some(0),
                need_power: Some(cloth.use_power1.first().copied().unwrap_or(0)),
                r#type: Some(BuffLayerType::Normal as i32),
            });
        }
        if cloth.skill2 != 0 {
            skills.push(PlayerSkillInfo {
                skill_id: Some(cloth.skill2),
                cd: Some(0),
                need_power: Some(cloth.use_power2.first().copied().unwrap_or(0)),
                r#type: Some(BuffLayerType::Normal as i32),
            });
        }
        if cloth.skill3 != 0 {
            skills.push(PlayerSkillInfo {
                skill_id: Some(cloth.skill3),
                cd: Some(0),
                need_power: None,
                r#type: Some(BuffLayerType::Normal as i32),
            });
        }

        skills
    }
}
