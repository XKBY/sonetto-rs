use sonettobuf::{FightStep, fight_step};

use crate::state::battle::fight_step::ActEffectBuilder;

/// Build the opening EFFECT step for a round:
///   USECARDS(159)    — player selected cards only, teamType=0
///   CARDSPUSH(154)   — remaining hand after selections, teamType=0
///   CARDDECKNUM(310) — remaining candidate pool size, teamType=1
pub fn build_refresh_step(
    selected_cards: Vec<sonettobuf::CardInfo>,
    remaining_hand: Vec<sonettobuf::CardInfo>,
    deck_num: i32,
) -> FightStep {
    FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        from_id: Some(0),
        to_id: Some(0),
        act_id: Some(0),
        act_effect: vec![
            ActEffectBuilder::use_cards(selected_cards),
            ActEffectBuilder::cards_push(remaining_hand, None),
            ActEffectBuilder::card_deck_num(deck_num),
        ],
        card_index: Some(0),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}
