use super::super::super::utils::moxie_change;
use sonettobuf::{ActEffect, effect_type_enum::EffectType};

pub fn add_ex_point(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![moxie_change(target, amount)]
}

pub fn bloodlust(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffect {
        effect_type: Some(EffectType::Bloodlust as i32),
        target_id: Some(target),
        effect_num: Some(amount),
        ..Default::default()
    }]
}

pub fn change_power(target: i64, amount: i32) -> Vec<ActEffect> {
    vec![ActEffect {
        effect_type: Some(EffectType::Powerchange as i32),
        target_id: Some(target),
        effect_num: Some(amount),
        config_effect: Some(1),
        ..Default::default()
    }]
}

pub fn average_life(target: i64) -> Vec<ActEffect> {
    vec![ActEffect {
        effect_type: Some(EffectType::Averagelife as i32),
        target_id: Some(target),
        ..Default::default()
    }]
}
