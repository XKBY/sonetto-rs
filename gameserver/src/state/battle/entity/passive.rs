use config::configs;
use database::models::game::heros::HeroData;
use std::collections::HashMap;

pub struct Passive;

impl Passive {
    pub fn get(
        hero_data: &HeroData,
        equip_id: Option<i32>,
        destiny: Option<&HashMap<i32, i32>>,
        destiny_stone: i32,
        destiny_rank: i32,
    ) -> Vec<i32> {
        let game = configs::get();
        let r = &hero_data.record;
        let hero_id = r.hero_id;
        let ex_level = r.ex_skill_level;

        let mut passives: Vec<i32> = Vec::new();

        // activity override base passives
        let activity_base = Self::get_activity_base(hero_id);
        let ex_map = Self::build_ex_map(hero_id, ex_level);

        if let Some(base_ids) = activity_base {
            for d in base_ids {
                let ex_resolved = *ex_map.get(&d).unwrap_or(&d);
                let final_id = destiny
                    .and_then(|m| m.get(&d))
                    .copied()
                    .unwrap_or(ex_resolved);
                passives.push(final_id);
            }
        } else {
            let mut rows: Vec<(i32, i32)> = game
                .skill_passive_level
                .iter()
                .filter(|s| s.hero_id == hero_id && s.skill_passive != 0)
                .map(|s| (s.skill_level, s.skill_passive))
                .collect();

            rows.sort_by_key(|(level, _)| if *level == 0 { i32::MAX } else { *level });

            for (_, id) in rows {
                // apply ex_level upgrades — replaces base passive with upgraded variant
                let upgraded = *ex_map.get(&id).unwrap_or(&id);
                let final_id = destiny
                    .and_then(|m| m.get(&id))
                    .copied()
                    .unwrap_or(upgraded);
                passives.push(final_id);
            }
        }

        // Destiny passives that have no config link — keyed by destinyStone ID.
        // These skills exist in skill_effect but are absent from skill_passive_level
        // and character_destiny_facets.exchangeSkills. battle3 replay reads the
        // passives off the input fight protobuf so these entries are for
        // fresh-battle builds.
        //
        // Each entry carries `(skill_id, tier_required)` so a low-rank hero
        // doesn't get higher-tier passives. Per-hero entries are migrating into
        // heroes/{name}.rs (Sotheby done); remaining heroes inline pending
        // their migrations.
        use crate::state::battle::heroes;
        let destiny_skills = if destiny_stone == heroes::sotheby::DESTINY_STONE {
            heroes::sotheby::destiny_passive_skills(destiny_stone, destiny_rank)
        } else {
            let inline_map: &[(i32, &[(i32, u8)])] = &[
                (306201, &[(30620144, 1), (30620147, 1)]),                       // 3062 Melania
                (306301, &[(30630151, 1), (30630161, 2), (30630171, 3)]),        // 3063 Pickles
                (308801, &[(308801911, 1), (308801921, 2), (308802111, 4)]),     // 3088 Semmelweis
            ];
            inline_map
                .iter()
                .find(|(s, _)| *s == destiny_stone)
                .map(|(_, entries)| {
                    entries
                        .iter()
                        .filter(|(_, tier)| destiny_rank >= *tier as i32)
                        .map(|(id, _)| *id)
                        .collect::<Vec<i32>>()
                })
                .unwrap_or_default()
        };
        for id in destiny_skills {
            if !passives.contains(&id) {
                passives.push(id);
            }
        }

        //Sentinel
        if hero_id == 3126 {
            let base = 31260191;
            let id = *ex_map.get(&base).unwrap_or(&base);
            if !passives.contains(&id) {
                passives.push(id);
            }
        }

        // equip passives
        if let Some(eid) = equip_id
            && let Some(e) = game.equip_skill.iter().find(|e| e.id == eid)
        {
            if e.skill != 0 {
                passives.push(e.skill);
            }
            if e.skill2 != 0 {
                passives.push(e.skill2);
            }
        }

        passives
    }

    fn get_activity_base(hero_id: i32) -> Option<Vec<i32>> {
        let game = configs::get();

        if let Some(r) = game.activity174_role.iter().find(|r| r.hero_id == hero_id)
            && !r.passive_skill.is_empty()
        {
            return Some(
                r.passive_skill
                    .split('|')
                    .filter_map(|v| v.parse().ok())
                    .collect(),
            );
        }

        if let Some(r) = game.activity191_role.iter().find(|r| r.role_id == hero_id)
            && !r.passive_skill.is_empty()
        {
            return Some(
                r.passive_skill
                    .split('|')
                    .filter_map(|v| v.parse().ok())
                    .collect(),
            );
        }

        None
    }

    fn build_ex_map(hero_id: i32, ex_level: i32) -> HashMap<i32, i32> {
        let game = configs::get();
        let mut map = HashMap::new();

        for lvl in 1..=ex_level {
            if let Some(ex) = game
                .skill_ex_level
                .iter()
                .find(|s| s.hero_id == hero_id && s.skill_level == lvl)
            {
                for pair in ex.passive_skill.split('|') {
                    if let Some((d, a)) = pair.split_once('#')
                        && let (Ok(d), Ok(a)) = (d.parse::<i32>(), a.parse::<i32>())
                    {
                        map.insert(d, a);
                    }
                }
            }
        }

        map
    }
}
