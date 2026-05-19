use std::collections::HashSet;
use rand::Rng;
use sonettobuf::{CardInfo, Fight};
use super::hand::{purge_dead_entity_cards, refill_hand};

#[derive(Default, Debug, Clone)]
pub struct DeckManager {
    pub player_hand: Vec<CardInfo>,
    pub player_deck: Vec<CardInfo>,
    pub player_ex_deck: Vec<CardInfo>,
    pub enemy_hand: Vec<CardInfo>,
    pub enemy_deck: Vec<CardInfo>,
    pub enemy_ex_deck: Vec<CardInfo>,
}

impl DeckManager {
    pub fn purge_player_dead_cards(&mut self, alive_uids: &HashSet<i64>) {
        purge_dead_entity_cards(&mut self.player_hand, alive_uids);
        purge_dead_entity_cards(&mut self.player_ex_deck, alive_uids);
    }

    pub fn purge_enemy_dead_cards(&mut self, alive_uids: &HashSet<i64>) {
        purge_dead_entity_cards(&mut self.enemy_hand, alive_uids);
        purge_dead_entity_cards(&mut self.enemy_ex_deck, alive_uids);
    }

    pub fn refill_player_hand(&mut self, rng: &mut impl Rng, alive_uids: &HashSet<i64>, extra: usize, fight: &Fight) -> Vec<CardInfo> {
        refill_hand(rng, &mut self.player_hand, &mut self.player_deck, &mut self.player_ex_deck, alive_uids, extra, fight)
    }

    pub fn refill_enemy_hand(&mut self, rng: &mut impl Rng, alive_uids: &HashSet<i64>, fight: &Fight) -> Vec<CardInfo> {
        refill_hand(rng, &mut self.enemy_hand, &mut self.enemy_deck, &mut self.enemy_ex_deck, alive_uids, 0, fight)
    }
}
