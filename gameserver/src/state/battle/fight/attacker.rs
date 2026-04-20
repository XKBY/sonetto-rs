use super::super::entity::builder::EntityBuilder;
use super::team::Team;
use anyhow::Result;
use database::models::game::heros::{HeroModel, UserHeroModel};
use sonettobuf::FightTeam;
use sqlx::SqlitePool;

pub struct Attacker;

impl Attacker {
    pub async fn get(
        pool: &SqlitePool,
        user_id: i64,
        fight_group: &sonettobuf::FightGroup,
    ) -> Result<FightTeam> {
        let mut entitys = Vec::new();
        let mut sub_entitys = Vec::new();
        let hero = UserHeroModel::new(user_id, pool.clone());

        for (position, hero_uid) in fight_group.hero_list.iter().enumerate() {
            if *hero_uid == 0 {
                continue;
            }

            let hero_data = hero.get_uid(*hero_uid as i32).await?;
            let equip = Self::fetch_equip(pool, &hero_data).await;

            let mut builder = EntityBuilder::new(hero_data, (position + 1) as i32, 1, false);
            if let Some(equip) = equip {
                builder = builder.with_equip(equip);
            }
            entitys.push(builder.build());
        }

        for hero_uid in fight_group.sub_hero_list.iter() {
            if *hero_uid == 0 {
                continue;
            }

            let hero_data = hero.get_uid(*hero_uid as i32).await?;
            let equip = Self::fetch_equip(pool, &hero_data).await;

            let mut builder = EntityBuilder::new(hero_data, -1, 1, true);
            if let Some(equip) = equip {
                builder = builder.with_equip(equip);
            }
            sub_entitys.push(builder.build());
        }

        let player_entity = EntityBuilder::player(user_id, 1);
        let skill_infos = Team::get_player_skills(fight_group.cloth_id);

        Ok(Team::build(
            entitys,
            sub_entitys,
            player_entity,
            Some(15),
            fight_group.cloth_id,
            skill_infos,
        ))
    }

    async fn fetch_equip(
        pool: &SqlitePool,
        hero_data: &database::models::game::heros::HeroData,
    ) -> Option<database::db::game::equipment::Equipment> {
        use database::models::game::equipment::UserEquipmentModel;

        UserEquipmentModel::new(hero_data.record.user_id, pool.clone())
            .get_equip(hero_data.record.default_equip_uid)
            .await
            .ok()
    }
}
