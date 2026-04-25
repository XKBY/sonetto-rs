use std::collections::HashSet;

use sonettobuf::{CardInfo, Fight, FightStep};

#[derive(Default, Debug, Clone)]
pub struct RoundState {
    pub act_point: i32,
    pub player_deck: Vec<CardInfo>,
    pub ai_cards: Vec<CardInfo>,
    pub ai_override_steps: Option<Vec<FightStep>>,
    pub used_cards: Vec<i32>,
    pub enemy_skill_actors: HashSet<i64>,
    pub move_num: i32,
    pub pending_cloth_power_delta: i32,
    pub is_finish: bool,
}

impl RoundState {
    pub fn new(_fight: &Fight) -> Self {
        Self {
            // AP is the playable card count budget (not team power).
            act_point: 3,
            ..Default::default()
        }
    }
}
