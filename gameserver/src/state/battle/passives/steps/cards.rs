use sonettobuf::{ActEffect, FightStep, effect_type_enum::EffectType};

use crate::state::battle::fight_step::FightStepBuilder;

pub fn build_deal_cards_step() -> FightStep {
    FightStepBuilder::effect()
        .with(ActEffect {
            effect_type: Some(EffectType::Enterfightdeal as i32),
            effect_num: Some(0),
            team_type: Some(0),
            target_id: Some(0),
            ..Default::default()
        })
        .build()
}

pub fn build_card_deck_num_step(attacker_count: usize) -> FightStep {
    let deck_size = attacker_count as i32 * 16;
    FightStepBuilder::effect()
        .with(ActEffect {
            effect_type: Some(EffectType::Carddecknum as i32),
            effect_num: Some(deck_size),
            team_type: Some(1),
            target_id: Some(0),
            ..Default::default()
        })
        .with(ActEffect {
            effect_type: Some(EffectType::Carddecknum as i32),
            effect_num: Some(deck_size),
            team_type: Some(1),
            target_id: Some(0),
            ..Default::default()
        })
        .build()
}

pub fn build_card_deck_num_final_step(attacker_count: usize) -> FightStep {
    let deck_size = attacker_count as i32 * 16;
    FightStepBuilder::effect()
        .with(ActEffect {
            effect_type: Some(EffectType::Carddecknum as i32),
            effect_num: Some(deck_size),
            team_type: Some(1),
            target_id: Some(0),
            ..Default::default()
        })
        .build()
}
