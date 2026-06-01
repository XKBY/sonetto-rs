use super::super::entity::skill::parse_skill_group;
use anyhow::Result;
use config::{configs, hero_trial::HeroTrial};
use sonettobuf::{EquipRecord, FightEntityInfo, HeroAttribute};

pub struct Trial;

impl Trial {
    /// Build a trial hero entity.
    ///
    /// `hero_uid` is the negative slot index from the client (-1 = slot 0, -2 = slot 1, …).
    /// `battle_id` is used to look up `battle.trial_heros` to pick the correct trial config.
    pub fn get(hero_uid: i64, battle_id: i32, position: i32, team_type: i32) -> Result<FightEntityInfo> {
        let game_data = configs::get();
        let slot = ((-hero_uid) - 1) as usize;

        // ── Step 1: log what the battle config says about trial heroes ──────────
        let battle_trial_heros = game_data
            .battle
            .iter()
            .find(|b| b.id == battle_id)
            .map(|b| b.trial_heros.clone())
            .unwrap_or_else(|| "<battle not found>".into());
        tracing::info!(
            "Trial::get uid={} battle_id={} slot={} battle.trial_heros={:?}",
            hero_uid, battle_id, slot, battle_trial_heros
        );

        // ── Step 2: resolve trial config ID from battle.trial_heros ─────────────
        let trial_id: i32 = if battle_id > 0 {
            game_data
                .battle
                .iter()
                .find(|b| b.id == battle_id)
                .and_then(|b| {
                    b.trial_heros
                        .split('|')
                        .filter_map(|entry| {
                            // Each entry is either just an ID ("1001") or
                            // ID#extra#position ("3122011#0#1") — take only the first token.
                            entry.split('#').next().and_then(|s| s.trim().parse::<i32>().ok())
                        })
                        .nth(slot)
                })
                .unwrap_or(0)
        } else {
            0
        };
        tracing::info!(
            "Trial::get uid={} slot={} → trial_id={}  ({})",
            hero_uid, slot, trial_id,
            if trial_id > 0 { "from battle.trial_heros" } else { "will use global fallback" }
        );

        // ── Step 3: load trial config by id, then by hero_id, then by index ─────
        let trial_data: &HeroTrial = if trial_id > 0 {
            // Primary: look up by trial config id
            if let Some(td) = game_data.hero_trial.get(trial_id) {
                tracing::info!(
                    "Trial::get uid={} found trial by id={} → hero_id={} level={}",
                    hero_uid, trial_id, td.hero_id, td.level
                );
                td
            } else {
                // Secondary: maybe trial_heros contains hero model IDs, not config IDs
                tracing::warn!(
                    "Trial::get uid={} hero_trial.get({}) returned None; \
                     trying lookup by hero_id instead",
                    hero_uid, trial_id
                );
                let by_hero = game_data
                    .hero_trial
                    .iter()
                    .find(|t| t.hero_id == trial_id);
                if let Some(td) = by_hero {
                    tracing::info!(
                        "Trial::get uid={} found trial by hero_id={} → config id={} level={}",
                        hero_uid, trial_id, td.id, td.level
                    );
                    td
                } else {
                    tracing::warn!(
                        "Trial::get uid={} no hero_trial id={} or hero_id={}; \
                         using unique-hero_id fallback for slot {}",
                        hero_uid, trial_id, trial_id, slot
                    );
                    let unique_trial: Option<&config::hero_trial::HeroTrial> = {
                        let mut seen = std::collections::HashSet::new();
                        game_data.hero_trial.iter().filter(|t| seen.insert(t.hero_id)).nth(slot)
                    };
                    unique_trial.ok_or_else(|| anyhow::anyhow!(
                        "No unique trial hero for slot {} (battle {})", slot, battle_id
                    ))?
                }
            }
        } else {
            // battle.trial_heros is empty — pick the N-th UNIQUE hero_id.
            // Plain .nth(slot) is wrong when the table has multiple entries for
            // the same character (Sonetto at level 100 AND 120 occupy slots 0+1,
            // so slot 1 would also return Sonetto instead of Apple).
            tracing::warn!(
                "Trial::get uid={} battle.trial_heros empty for slot {}; \
                 using unique-hero_id fallback (slot {})",
                hero_uid, slot, slot
            );
            for (i, t) in game_data.hero_trial.iter().take(6).enumerate() {
                tracing::info!(
                    "Trial::get  global hero_trial[{}]: id={} hero_id={} level={}",
                    i, t.id, t.hero_id, t.level
                );
            }
            let unique_trial: Option<&config::hero_trial::HeroTrial> = {
                let mut seen = std::collections::HashSet::new();
                game_data.hero_trial.iter().filter(|t| seen.insert(t.hero_id)).nth(slot)
            };
            unique_trial.ok_or_else(|| anyhow::anyhow!(
                "No unique trial hero for slot {} (battle {})", slot, battle_id
            ))?
        };

        // ── Step 4: load character config ────────────────────────────────────────
        let hero_config = game_data
            .character
            .iter()
            .find(|h| h.id == trial_data.hero_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No character config for hero_id {} (trial id={})",
                    trial_data.hero_id, trial_data.id
                )
            })?;

        // ── Step 5: get stats (with max-level fallback) ──────────────────────────
        let (hp, attack, defense, mdefense, technic) = Self::get_stats(trial_data)?;

        let skill_group1 = parse_skill_group(&hero_config.skill, 1);
        let skill_group2 = parse_skill_group(&hero_config.skill, 2);

        tracing::info!(
            "Trial::get uid={} → entity model_id={} level={} hp={} atk={} def={}",
            hero_uid, trial_data.hero_id, trial_data.level, hp, attack, defense
        );

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

        // ── Prefer exact level match ─────────────────────────────────────────────
        if let Some(ld) = game_data
            .character_level
            .iter()
            .find(|c| c.hero_id == trial_data.hero_id && c.level == trial_data.level)
        {
            return Ok((ld.hp, ld.atk, ld.def, ld.mdef, ld.technic));
        }

        // ── Fall back to MAX available level (much better than falling to level 1)
        let max_ld = game_data
            .character_level
            .iter()
            .filter(|c| c.hero_id == trial_data.hero_id)
            .max_by_key(|c| c.level);

        if let Some(ld) = max_ld {
            tracing::warn!(
                "Trial get_stats: level {} not found for hero {}, \
                 using max available level {} (hp={})",
                trial_data.level, trial_data.hero_id, ld.level, ld.hp
            );
            return Ok((ld.hp, ld.atk, ld.def, ld.mdef, ld.technic));
        }

        Err(anyhow::anyhow!(
            "No character_level data at all for hero_id {}",
            trial_data.hero_id
        ))
    }
}