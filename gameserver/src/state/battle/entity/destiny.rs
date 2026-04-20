use config::configs;
use std::collections::HashMap;

pub struct Destiny;

impl Destiny {
    pub fn get(facets_id: i32, rank: i32) -> Option<HashMap<i32, i32>> {
        if facets_id <= 0 || rank <= 0 {
            return None;
        }

        let game = configs::get();

        game.character_destiny_facets
            .iter()
            .find(|f| f.facets_id == facets_id && f.level == rank)
            .map(|f| Self::parse_exchange(&f.exchange_skills))
    }

    fn parse_exchange(s: &str) -> HashMap<i32, i32> {
        let mut map = HashMap::new();

        for pair in s.split('|') {
            if let Some((old, new)) = pair.split_once('#')
                && let (Ok(o), Ok(n)) = (old.parse(), new.parse())
            {
                map.insert(o, n);
            }
        }

        map
    }
}
