use super::super::entity::builder::EntityBuilder;
use super::super::fight::trial::Trial;
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
        battle_id: i32,
        fight_group: &sonettobuf::FightGroup,
    ) -> Result<FightTeam> {
        let mut entitys = Vec::new();
        let mut sub_entitys = Vec::new();
        let hero = UserHeroModel::new(user_id, pool.clone());

        tracing::info!(
            "Attacker::get battle_id={} hero_list={:?} sub_hero_list={:?}",
            battle_id, fight_group.hero_list, fight_group.sub_hero_list
        );

        for (position, hero_uid) in fight_group.hero_list.iter().enumerate() {
            if *hero_uid == 0 {
                continue;
            }
            let entity = if *hero_uid < 0 {
                let e = Trial::get(*hero_uid, battle_id, (position + 1) as i32, 1)?;
                tracing::info!(
                    "Attacker: trial hero uid={} → model_id={:?} position={:?} hp={:?} level={:?}",
                    hero_uid, e.model_id, e.position, e.current_hp, e.level
                );
                e
            } else {
                let hero_data = hero.get_uid(*hero_uid as i32).await?;
                tracing::info!(
                    "Attacker: player hero uid={} → hero_id={} level={}",
                    hero_uid, hero_data.record.hero_id, hero_data.record.level
                );
                let equip = Self::fetch_equip(pool, &hero_data).await;
                let mut builder = EntityBuilder::new(hero_data, (position + 1) as i32, 1, false);
                if let Some(equip) = equip {
                    builder = builder.with_equip(equip);
                }
                builder.build()
            };
            entitys.push(entity);
        }

        for hero_uid in fight_group.sub_hero_list.iter() {
            if *hero_uid == 0 {
                continue;
            }
            let entity = if *hero_uid < 0 {
                let e = Trial::get(*hero_uid, battle_id, -1, 1)?;
                tracing::info!(
                    "Attacker: trial support uid={} → model_id={:?} hp={:?} level={:?}",
                    hero_uid, e.model_id, e.current_hp, e.level
                );
                e
            } else {
                let hero_data = hero.get_uid(*hero_uid as i32).await?;
                let equip = Self::fetch_equip(pool, &hero_data).await;
                let mut builder = EntityBuilder::new(hero_data, -1, 1, true);
                if let Some(equip) = equip {
                    builder = builder.with_equip(equip);
                }
                builder.build()
            };
            sub_entitys.push(entity);
        }

        tracing::info!(
            "Attacker::get complete: {} main + {} sub entities",
            entitys.len(), sub_entitys.len()
        );

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