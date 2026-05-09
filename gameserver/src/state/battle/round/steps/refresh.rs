use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::{fight_step::ActEffectBuilder, types::effects::EffectType};

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
            ActEffect {
                effect_type: Some(EffectType::UseCards as i32),
                card_info_list: selected_cards,
                ..Default::default()
            },
            ActEffect {
                effect_type: Some(EffectType::CardsPush as i32),
                card_info_list: remaining_hand,
                ..Default::default()
            },
            ActEffectBuilder::card_deck_num(deck_num),
        ],
        card_index: Some(0),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}
