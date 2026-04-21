use super::super::super::utils::moxie_change;
use crate::state::battle::fight_step::ActEffectBuilder;
use sonettobuf::{ActEffect, effect_type_enum::EffectType};

pub fn add_ex_point(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![moxie_change(target, amount)]
}

pub fn bloodlust(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![
        ActEffectBuilder::new(EffectType::Bloodlust as i32, target)
            .effect_num(amount)
            .build(),
    ]
}

pub fn change_power(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![
        ActEffectBuilder::new(EffectType::Powerchange as i32, target)
            .effect_num(amount)
            .config_effect(1)
            .build(),
    ]
}

pub fn average_life(target: i64) -> Vec<ActEffect> {
    vec![ActEffectBuilder::new(EffectType::Averagelife as i32, target).build()]
}
