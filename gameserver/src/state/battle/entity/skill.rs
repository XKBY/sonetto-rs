use config::configs;
use database::models::game::heros::HeroData;
use std::collections::HashMap;

pub struct Skill;

impl Skill {
    pub fn get(
        hero_data: &HeroData,
        is_sub: bool,
        destiny: Option<&HashMap<i32, i32>>,
    ) -> (Vec<i32>, Vec<i32>) {
        let r = &hero_data.record;
        let game = configs::get();

        let hero_type = game
            .character
            .iter()
            .find(|c| c.id == r.hero_id)
            .map(|c| c.hero_type)
            .unwrap_or(1);

        let (mut sg1, mut sg2) = if let Some((sg1, sg2, _)) = Self::get_activity174_kit(r.hero_id) {
            (sg1, sg2)
        } else if is_sub {
            (
                Self::get_from_character(r.hero_id, 1),
                Self::get_from_character(r.hero_id, 2),
            )
        } else {
            (
                Self::get_group(r.hero_id, 1, r.ex_skill_level, hero_type),
                Self::get_group(r.hero_id, 2, r.ex_skill_level, hero_type),
            )
        };

        if let Some(map) = destiny {
            Self::apply_exchange(&mut sg1, map);
            Self::apply_exchange(&mut sg2, map);
        }

        (sg1, sg2)
    }

    pub fn get_ex(hero_data: &HeroData, destiny: Option<&HashMap<i32, i32>>) -> i32 {
        let game = configs::get();
        let r = &hero_data.record;

        // special logic for nautika
        let mut ex = if r.hero_id == 3120 {
            let ex = game
                .skill_ex_level
                .iter()
                .find(|s| s.hero_id == r.hero_id && s.skill_level == r.ex_skill_level)
                .map(|s| s.skill_ex)
                .unwrap_or(0);
            if ex == 0 {
                game.skill_ex_level
                    .iter()
                    .find(|s| s.hero_id == r.hero_id && s.skill_level == 1)
                    .map(|s| s.skill_ex)
                    .unwrap_or(0)
            } else {
                ex
            }
        } else {
            game.character
                .iter()
                .find(|c| c.id == r.hero_id)
                .map(|c| c.ex_skill)
                .unwrap_or(0)
        };

        if let Some(map) = destiny
            && let Some(replaced) = map.get(&ex)
        {
            ex = *replaced;
        }

        ex
    }

    fn get_group(hero_id: i32, group: i32, ex_level: i32, hero_type: i32) -> Vec<i32> {
        // special logic for nautika
        if hero_id == 3120 {
            let ex = Self::lookup_ex_group(hero_id, group, ex_level);
            if !ex.is_empty() {
                let raw_ids = Self::parse_ex_string(ex, hero_type);
                // Resolve each skill ID through the skill_effect mapping so cards
                // carry executable skill IDs rather than ex-level placeholders.
                return raw_ids
                    .into_iter()
                    .map(super::super::skill::cache::resolve_skill_effect_id)
                    .collect();
            }
        }
        Self::get_from_character(hero_id, group)
    }

    #[allow(dead_code)]
    pub fn get_skill_groups(hero_id: i32, ex_level: i32) -> (Vec<i32>, Vec<i32>) {
        Self::get_skill_groups_with_destiny(hero_id, ex_level, None)
    }

    pub fn get_skill_groups_with_destiny(
        hero_id: i32,
        ex_level: i32,
        destiny: Option<&HashMap<i32, i32>>,
    ) -> (Vec<i32>, Vec<i32>) {
        let game = configs::get();
        let hero_type = game
            .character
            .iter()
            .find(|c| c.id == hero_id)
            .map(|c| c.hero_type)
            .unwrap_or(1);

        let mut sg1 = Skill::get_group(hero_id, 1, ex_level, hero_type);
        let mut sg2 = Skill::get_group(hero_id, 2, ex_level, hero_type);

        if let Some(map) = destiny {
            Self::apply_exchange(&mut sg1, map);
            Self::apply_exchange(&mut sg2, map);
        }

        (sg1, sg2)
    }

    // this gets the nexxt skill level. not implemented yet but we'll use it later to track merges
    #[allow(dead_code)]
    /// For a choice card skill_id, find which skill_ex_level entry contains it,
    /// determine the rank index (pipe position), and return all other comma-groups
    /// at the same rank — these are the selectable options.
    pub fn get_choice_options(hero_id: i32, choice_skill_id: i32, ex_level: i32) -> Vec<i32> {
        let game = config::configs::get();

        for lvl in (1..=ex_level).rev() {
            let Some(ex) = game
                .skill_ex_level
                .iter()
                .find(|s| s.hero_id == hero_id && s.skill_level == lvl)
            else {
                continue;
            };

            if ex.skill_group2.is_empty() {
                continue;
            }

            // comma-groups = options, pipes = ranks within each option
            let groups: Vec<Vec<i32>> = ex
                .skill_group2
                .split(',')
                .map(|g| g.split('|').filter_map(|v| v.parse().ok()).collect())
                .collect();

            // Find which group contains our choice card and at what rank index
            let mut rank_idx = None;
            for group in &groups {
                if let Some(pos) = group.iter().position(|&id| id == choice_skill_id) {
                    rank_idx = Some(pos);
                    break;
                }
            }

            let Some(rank) = rank_idx else { continue };

            // Return all OTHER groups at the same rank index (skip the choice card group)
            let options: Vec<i32> = groups
                .iter()
                .filter(|g| !g.contains(&choice_skill_id))
                .filter_map(|g| g.get(rank).copied())
                .collect();

            if !options.is_empty() {
                return options;
            }
        }

        vec![]
    }

    #[allow(dead_code)]
    pub fn resolve_to_ex_level(base_skill: i32, ex_level: i32) -> i32 {
        let offset = match ex_level {
            1..=3 => 0,
            4 => 1,
            5 => 2,
            _ => 0,
        };
        base_skill + offset
    }

    #[allow(dead_code)]
    pub fn get_next_skill_id(hero_id: i32, skill_id: i32) -> Option<i32> {
        let game = configs::get();
        let character = game.character.iter().find(|c| c.id == hero_id)?;

        for group_str in character.skill.split('|') {
            let parts: Vec<i32> = group_str
                .split('#')
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect();

            if let Some(pos) = parts.iter().position(|&s| s == skill_id) {
                return parts.get(pos + 1).copied();
            }
        }
        None
    }

    fn parse_ex_string(s: &str, hero_type: i32) -> Vec<i32> {
        if s.contains(',') {
            let sets: Vec<&str> = s.split(',').collect();
            let idx = if hero_type < 1 || hero_type as usize > sets.len() {
                0
            } else {
                (hero_type - 1) as usize
            };

            sets.get(idx)
                .into_iter()
                .flat_map(|v| v.split('|'))
                .filter_map(|v| v.parse().ok())
                .collect()
        } else {
            s.split('|').filter_map(|v| v.parse().ok()).collect()
        }
    }

    fn get_from_character(hero_id: i32, group: i32) -> Vec<i32> {
        let game = configs::get();
        let Some(c) = game.character.iter().find(|c| c.id == hero_id) else {
            tracing::warn!("Character {} not found", hero_id);
            return vec![];
        };
        parse_skill_group(&c.skill, group)
    }

    fn lookup_ex_group(hero_id: i32, group: i32, ex_level: i32) -> &'static str {
        let game = configs::get();

        for lvl in (1..=ex_level).rev() {
            if let Some(ex) = game
                .skill_ex_level
                .iter()
                .find(|s| s.hero_id == hero_id && s.skill_level == lvl)
            {
                match group {
                    1 if !ex.skill_group1.is_empty() => return &ex.skill_group1,
                    2 if !ex.skill_group2.is_empty() => return &ex.skill_group2,
                    _ => {}
                }
            }
        }

        ""
    }

    fn get_activity174_kit(hero_id: i32) -> Option<(Vec<i32>, Vec<i32>, Vec<i32>)> {
        let game = configs::get();
        let role = game
            .activity174_role
            .iter()
            .find(|r| r.hero_id == hero_id)?;

        let sg1 = role
            .active_skill1
            .split('#')
            .filter_map(|v| v.parse().ok())
            .collect();
        let sg2 = role
            .active_skill2
            .split('#')
            .filter_map(|v| v.parse().ok())
            .collect();
        let passives = role
            .passive_skill
            .split('|')
            .filter_map(|v| v.parse().ok())
            .collect();

        Some((sg1, sg2, passives))
    }

    fn apply_exchange(list: &mut [i32], map: &HashMap<i32, i32>) {
        for v in list.iter_mut() {
            if let Some(new) = map.get(v) {
                *v = *new;
            }
        }
    }
}

pub fn parse_skill_group(skill_str: &str, target_group: i32) -> Vec<i32> {
    for group_str in skill_str.split('|') {
        let mut parts = group_str.split('#');
        let Some(first) = parts.next() else { continue };
        let Ok(group_num) = first.parse::<i32>() else {
            continue;
        };

        if group_num == target_group {
            return parts.filter_map(|s| s.parse::<i32>().ok()).collect();
        }
    }
    vec![]
}
