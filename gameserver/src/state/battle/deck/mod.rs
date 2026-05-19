mod draw;
mod manager;
mod opening;
pub(crate) mod pool;
mod hand;

pub use hand::{default_max_ap, generate_ai_deck, generate_initial_enemy_hand, generate_initial_hand};
pub(crate) use hand::{card_limit, purge_dead_entity_cards, refill_hand};
pub use manager::DeckManager;
pub use pool::{build_enemy_deck, build_player_deck, generate_deck, make_card};
pub use opening::apply_opening_deck;
pub(crate) use draw::draw_deck_guaranteed_by_uid_with_rng;
