#![allow(dead_code)]

//! Classifies where a skill comes from in hero kit data.
//! This is separate from emit routing and stays read-only after init.
//! The cache is built once from config tables plus known destiny passives.
//! Unknown covers anything outside the supported hero-kit sources.

use std::collections::{HashMap, HashSet};

use config::configs;
use once_cell::sync::Lazy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    /// Active incantation, slot 1 or 2, ranks 1-3.
    BasicIncantation { hero_id: i32, slot: u8, rank: u8 },
    /// EX incantation (`character.exSkill`).
    ExIncantation { hero_id: i32 },
    /// Form-shift / channel skill from Nautika's rank-replace overlay.
    ChannelSkill { hero_id: i32 },
    /// Standard hero passive listed in `skill_passive_level`.
    InsightPassive { hero_id: i32, tier: u8 },
    /// Euphoria tier replacing another skill in `exchangeSkills`.
    EuphoriaSwap { hero_id: i32, tier: u8, from: i32 },
    /// Euphoria passive with no `skill_passive_level` row.
    EuphoriaPassive { hero_id: i32, tier: u8 },
    /// Psychube skill from `equip_skill.json`.
    PsychubeSkill { equip_id: i32 },
    /// Psychube portray-level skill when config exposes it directly.
    PortraySkill { equip_id: i32, level: u8 },
    /// Couldn't classify.
    Unknown,
}

static SKILL_SOURCE_CACHE: Lazy<HashMap<i32, SkillSource>> = Lazy::new(build_cache);

pub fn classify(skill_id: i32) -> SkillSource {
    SKILL_SOURCE_CACHE
        .get(&skill_id)
        .copied()
        .unwrap_or(SkillSource::Unknown)
}

pub fn owning_hero(skill_id: i32) -> Option<i32> {
    match classify(skill_id) {
        SkillSource::BasicIncantation { hero_id, .. }
        | SkillSource::ExIncantation { hero_id }
        | SkillSource::ChannelSkill { hero_id }
        | SkillSource::InsightPassive { hero_id, .. }
        | SkillSource::EuphoriaSwap { hero_id, .. }
        | SkillSource::EuphoriaPassive { hero_id, .. } => Some(hero_id),
        SkillSource::PsychubeSkill { .. }
        | SkillSource::PortraySkill { .. }
        | SkillSource::Unknown => None,
    }
}

pub fn is_insight_passive(skill_id: i32) -> bool {
    matches!(classify(skill_id), SkillSource::InsightPassive { .. })
}

pub fn is_euphoria_origin(skill_id: i32) -> bool {
    matches!(
        classify(skill_id),
        SkillSource::EuphoriaSwap { .. } | SkillSource::EuphoriaPassive { .. }
    )
}

fn build_cache() -> HashMap<i32, SkillSource> {
    let game = configs::get();

    let insight_passives = build_insight_passives();
    let basic_incantations = build_basic_incantations();
    let channel_skills = build_channel_skills();
    let ex_incantations = build_ex_incantations();
    let psychube_skills = build_psychube_skills();
    let euphoria_swaps = build_euphoria_swaps();
    let euphoria_passives = build_euphoria_passives(&insight_passives);

    let mut cache = HashMap::with_capacity(
        game.skill_passive_level.len()
            + game.character.len() * 8
            + game.equip_skill.len() * 2
            + game.character_destiny_facets.len() * 8,
    );

    merge_priority(&mut cache, euphoria_swaps);
    merge_priority(&mut cache, euphoria_passives);
    merge_priority(&mut cache, insight_passives);
    merge_priority(&mut cache, basic_incantations);
    merge_priority(&mut cache, channel_skills);
    merge_priority(&mut cache, ex_incantations);
    merge_priority(&mut cache, psychube_skills);

    cache
}

fn merge_priority(cache: &mut HashMap<i32, SkillSource>, entries: HashMap<i32, SkillSource>) {
    for (skill_id, source) in entries {
        cache.entry(skill_id).or_insert(source);
    }
}

fn build_basic_incantations() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let overlays: HashSet<i32> = game
        .character_rank_replace
        .iter()
        .map(|row| row.id)
        .collect();
    let mut cache = HashMap::new();

    for character in game.character.iter() {
        if overlays.contains(&character.id) {
            continue;
        }

        add_basic_group(&mut cache, character.id, &character.skill, 1);
        add_basic_group(&mut cache, character.id, &character.skill, 2);
    }

    for overlay in game.character_rank_replace.iter() {
        let variants = parse_group_variants(&overlay.skill, 2);

        add_basic_group(&mut cache, overlay.id, &overlay.skill, 1);
        if let Some(primary_form) = variants.first() {
            insert_basic_ranks(&mut cache, overlay.id, 2, primary_form);
        }
    }

    cache
}

fn build_channel_skills() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let mut cache = HashMap::new();

    for overlay in game.character_rank_replace.iter() {
        for variant in parse_group_variants(&overlay.skill, 2).iter().skip(1) {
            for &skill_id in variant {
                if skill_id > 0 {
                    cache.insert(
                        skill_id,
                        SkillSource::ChannelSkill {
                            hero_id: overlay.id,
                        },
                    );
                }
            }
        }
    }

    cache
}

fn build_ex_incantations() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let overlays: HashMap<i32, i32> = game
        .character_rank_replace
        .iter()
        .map(|row| (row.id, row.ex_skill))
        .collect();
    let mut cache = HashMap::new();

    for character in game.character.iter() {
        let ex_skill = overlays
            .get(&character.id)
            .copied()
            .unwrap_or(character.ex_skill);

        if ex_skill > 0 {
            cache.insert(
                ex_skill,
                SkillSource::ExIncantation {
                    hero_id: character.id,
                },
            );
        }
    }

    cache
}

fn build_insight_passives() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let mut cache = HashMap::new();

    for row in game.skill_passive_level.iter() {
        if row.skill_passive <= 0 {
            continue;
        }

        let tier = row.skill_level.max(0) as u8;
        match cache.get(&row.skill_passive).copied() {
            Some(SkillSource::InsightPassive {
                tier: current_tier, ..
            }) if current_tier > tier => {}
            _ => {
                cache.insert(
                    row.skill_passive,
                    SkillSource::InsightPassive {
                        hero_id: row.hero_id,
                        tier,
                    },
                );
            }
        }
    }

    cache
}

fn build_euphoria_swaps() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let hero_by_facets_id: HashMap<i32, i32> = game
        .character_destiny
        .iter()
        .filter_map(|row| {
            row.facets_id
                .split('#')
                .next()
                .and_then(|value| value.parse::<i32>().ok())
                .map(|facets_id| (facets_id, row.hero_id))
        })
        .collect();

    let mut cache = HashMap::new();

    for row in game.character_destiny_facets.iter() {
        let Some(hero_id) = hero_by_facets_id
            .get(&row.facets_id)
            .copied()
            .or_else(|| (row.facets_id > 0).then_some(row.facets_id / 100))
        else {
            continue;
        };

        let tier = row.level.max(0) as u8;
        for (from, to) in parse_exchange_pairs(&row.exchange_skills) {
            match cache.get(&to).copied() {
                Some(SkillSource::EuphoriaSwap {
                    tier: current_tier, ..
                }) if current_tier > tier => {}
                _ => {
                    cache.insert(
                        to,
                        SkillSource::EuphoriaSwap {
                            hero_id,
                            tier,
                            from,
                        },
                    );
                }
            }
        }
    }

    cache
}

fn build_euphoria_passives(
    insight_passives: &HashMap<i32, SkillSource>,
) -> HashMap<i32, SkillSource> {
    let mut cache = HashMap::new();

    for &(destiny_stone, entries) in destiny_passive_map() {
        let hero_id = destiny_stone / 100;
        for &(skill_id, tier) in entries {
            if insight_passives.contains_key(&skill_id) {
                continue;
            }

            cache.insert(skill_id, SkillSource::EuphoriaPassive { hero_id, tier });
        }
    }

    cache
}

fn build_psychube_skills() -> HashMap<i32, SkillSource> {
    let game = configs::get();
    let equip_id_by_skill_type: HashMap<i32, i32> = game
        .equip
        .iter()
        .filter(|row| row.skill_type > 0)
        .map(|row| (row.skill_type, row.id))
        .collect();
    let mut cache = HashMap::new();

    for row in game.equip_skill.iter() {
        let equip_id = equip_id_by_skill_type
            .get(&row.id)
            .copied()
            .unwrap_or(row.id);
        for skill_id in [row.skill, row.skill2] {
            if skill_id > 0 {
                cache.insert(skill_id, SkillSource::PsychubeSkill { equip_id });
            }
        }
    }

    cache
}

fn add_basic_group(cache: &mut HashMap<i32, SkillSource>, hero_id: i32, skill: &str, slot: u8) {
    if let Some(ranks) = parse_group_variants(skill, slot as i32).first() {
        insert_basic_ranks(cache, hero_id, slot, ranks);
    }
}

fn insert_basic_ranks(
    cache: &mut HashMap<i32, SkillSource>,
    hero_id: i32,
    slot: u8,
    ranks: &[i32],
) {
    for (idx, &skill_id) in ranks.iter().enumerate() {
        if skill_id > 0 {
            cache.insert(
                skill_id,
                SkillSource::BasicIncantation {
                    hero_id,
                    slot,
                    rank: (idx + 1) as u8,
                },
            );
        }
    }
}

fn parse_group_variants(skill: &str, target_group: i32) -> Vec<Vec<i32>> {
    for group in skill.split('|') {
        let Some((group_id, rest)) = group.split_once('#') else {
            continue;
        };
        let Ok(group_id) = group_id.parse::<i32>() else {
            continue;
        };
        if group_id != target_group {
            continue;
        }

        return rest
            .split(',')
            .map(|variant| {
                variant
                    .split('#')
                    .filter_map(|value| value.parse::<i32>().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|variant| !variant.is_empty())
            .collect();
    }

    Vec::new()
}

fn parse_exchange_pairs(exchange_skills: &str) -> Vec<(i32, i32)> {
    exchange_skills
        .split('|')
        .filter_map(|pair| {
            let (from, to) = pair.split_once('#')?;
            let from = from.parse::<i32>().ok()?;
            let to = to.parse::<i32>().ok()?;
            Some((from, to))
        })
        .collect()
}

fn destiny_passive_map() -> &'static [(i32, &'static [(i32, u8)])] {
    &[
        (300901, &[(30090144, 4), (30090145, 4), (30090146, 4)]),
        (306201, &[(30620144, 1), (30620147, 1)]),
        (306301, &[(30630151, 1), (30630161, 2), (30630171, 3)]),
        (308801, &[(308801911, 1), (308801921, 2), (308802111, 4)]),
    ]
}

#[cfg(test)]
fn cache_len() -> usize {
    SKILL_SOURCE_CACHE.len()
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Once};

    use super::*;

    static TEST_CONFIG_INIT: Once = Once::new();

    fn ensure_game_data_initialized() {
        TEST_CONFIG_INIT.call_once(|| {
            if config::configs::try_get().is_some() {
                return;
            }

            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let excel_dir = root.join("data").join("excel2json");
            if excel_dir.exists()
                && let Some(path) = excel_dir.to_str()
            {
                let _ = config::configs::init(path);
            }
        });
    }

    #[test]
    fn sotheby_basic_classifies() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(30090111),
            SkillSource::BasicIncantation {
                hero_id: 3009,
                slot: 1,
                rank: 1,
            }
        );
    }

    #[test]
    fn sotheby_ex_classifies() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(30090131),
            SkillSource::ExIncantation { hero_id: 3009 }
        );
    }

    #[test]
    fn nautika_alt_form_classifies_as_channel() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(31200161),
            SkillSource::ChannelSkill { hero_id: 3120 }
        );
    }

    #[test]
    fn insight_passive_classifies() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(30090141),
            SkillSource::InsightPassive {
                hero_id: 3009,
                tier: 1,
            }
        );
        assert!(is_insight_passive(30090141));
    }

    #[test]
    fn euphoria_swap_uses_highest_tier() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(300901431),
            SkillSource::EuphoriaSwap {
                hero_id: 3009,
                tier: 4,
                from: 30090143,
            }
        );
        assert!(is_euphoria_origin(300901431));
    }

    #[test]
    fn semmelweis_euphoria_passive() {
        ensure_game_data_initialized();
        match classify(308802111) {
            SkillSource::EuphoriaPassive { hero_id: 3088, .. } => {}
            other => panic!("expected EuphoriaPassive, got {other:?}"),
        }
    }

    #[test]
    fn psychube_skill_classifies() {
        ensure_game_data_initialized();
        assert_eq!(
            classify(430111),
            SkillSource::PsychubeSkill { equip_id: 1501 }
        );
    }

    #[test]
    fn owning_hero_returns_none_for_psychube() {
        ensure_game_data_initialized();
        assert_eq!(owning_hero(430111), None);
    }

    #[test]
    fn cache_is_populated() {
        ensure_game_data_initialized();
        assert!(cache_len() > 1000);
    }

    #[test]
    fn unknown_skill_returns_unknown() {
        ensure_game_data_initialized();
        assert_eq!(classify(99999999), SkillSource::Unknown);
    }
}
