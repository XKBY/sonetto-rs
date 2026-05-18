mod deck;
mod draw;
mod op;
mod opening;
pub(crate) mod pool;
mod upgrade;

pub use deck::{default_max_ap, generate_ai_deck, generate_initial_enemy_hand, generate_initial_hand};
pub(crate) use deck::{purge_dead_entity_cards, refill_hand};
pub use pool::{build_enemy_deck, build_player_deck, generate_deck, make_card};
pub use op::CardOpType;
pub use opening::apply_opening_deck;
pub use upgrade::apply_card_upgrades;

pub fn skill_level(skill_id: i32, entities: &[sonettobuf::FightEntityInfo]) -> usize {
    entities.iter().flat_map(|e| [&e.skill_group1, &e.skill_group2]).find_map(|g| {
        g.iter().position(|&id| id == skill_id)
    }).unwrap_or(0)
}
