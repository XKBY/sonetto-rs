use sonettobuf::FightStep;

use crate::state::battle::fight_step::FightStepBuilder;

pub fn build_deal_cards_step() -> FightStep {
    FightStepBuilder::effect()
        .with(crate::state::battle::fight_step::ActEffectBuilder::enter_fight_deal())
        .build()
}

pub fn build_card_deck_num_step(attacker_count: usize) -> FightStep {
    let deck_size = attacker_count as i32 * 16;
    FightStepBuilder::effect()
        .with(crate::state::battle::fight_step::ActEffectBuilder::card_deck_num(deck_size))
        .with(crate::state::battle::fight_step::ActEffectBuilder::card_deck_num(deck_size))
        .build()
}

pub fn build_card_deck_num_final_step(attacker_count: usize) -> FightStep {
    let deck_size = attacker_count as i32 * 16;
    FightStepBuilder::effect()
        .with(crate::state::battle::fight_step::ActEffectBuilder::card_deck_num(deck_size))
        .build()
}
