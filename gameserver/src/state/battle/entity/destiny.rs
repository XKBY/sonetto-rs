use config::configs;
use once_cell::sync::Lazy;
use std::collections::HashMap;

pub struct Destiny;

static DESTINY_CACHE: Lazy<HashMap<(i32, i32), HashMap<i32, i32>>> = Lazy::new(|| {
    let game = configs::get();
    let mut grouped: HashMap<i32, Vec<(i32, &str)>> = HashMap::new();

    for row in game.character_destiny_facets.iter() {
        if row.facets_id > 0 && row.level > 0 {
            grouped
                .entry(row.facets_id)
                .or_default()
                .push((row.level, row.exchange_skills.as_str()));
        }
    }

    let mut cache = HashMap::new();

    for (facets_id, rows) in grouped {
        let mut rows = rows;
        rows.sort_by_key(|(level, _)| *level);

        let mut cumulative = HashMap::new();
        let mut next_gap_level = 1;

        for (level, exchange_skills) in rows {
            while next_gap_level < level {
                if !cumulative.is_empty() {
                    cache.insert((facets_id, next_gap_level), cumulative.clone());
                }
                next_gap_level += 1;
            }

            Destiny::parse_exchange_into(exchange_skills, &mut cumulative);
            cache.insert((facets_id, level), cumulative.clone());
            next_gap_level = level + 1;
        }
    }

    cache
});

impl Destiny {
    pub fn get(facets_id: i32, rank: i32) -> Option<HashMap<i32, i32>> {
        Self::get_ref(facets_id, rank).cloned()
    }

    pub fn get_ref(facets_id: i32, rank: i32) -> Option<&'static HashMap<i32, i32>> {
        if facets_id <= 0 || rank <= 0 {
            return None;
        }

        DESTINY_CACHE.get(&(facets_id, rank)).or_else(|| {
            DESTINY_CACHE
                .iter()
                .filter_map(|(&(id, level), map)| {
                    (id == facets_id && level <= rank).then_some((level, map))
                })
                .max_by_key(|(level, _)| *level)
                .map(|(_, map)| map)
        })
    }

    pub fn facets_id_for_hero(hero_id: i32) -> Option<i32> {
        configs::get()
            .character_destiny
            .iter()
            .find(|row| row.hero_id == hero_id)
            .and_then(|row| row.facets_id.split('#').next())
            .and_then(|value| value.parse::<i32>().ok())
    }

    pub fn resolve_skill_id(facets_id: i32, rank: i32, skill_id: i32) -> i32 {
        Self::get_ref(facets_id, rank)
            .and_then(|map| map.get(&skill_id).copied())
            .unwrap_or(skill_id)
    }

    fn parse_exchange_into(s: &str, map: &mut HashMap<i32, i32>) {
        for pair in s.split('|') {
            if let Some((old, new)) = pair.split_once('#')
                && let (Ok(o), Ok(n)) = (old.parse(), new.parse())
            {
                map.insert(o, n);
            }
        }
    }
}
