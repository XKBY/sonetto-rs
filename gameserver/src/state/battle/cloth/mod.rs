pub mod first_melody;

use crate::state::battle::card::CardOpType;
use sonettobuf::{BeginRoundOper, Fight};

pub fn active_cloth_level(fight: &Fight) -> Option<config::cloth_level::ClothLevel> {
    let cloth_id = fight.attacker.as_ref().and_then(|a| a.cloth_id)?;
    config::configs::get()
        .cloth_level
        .iter()
        .find(|c| c.id == cloth_id && c.level == 1)
        .cloned()
}

pub fn parse_cloth_recover_delta(recover: &str, round_index: i32) -> i32 {
    recover
        .split('|')
        .filter_map(|entry| {
            let mut parts = entry.trim().split('#');
            let start_round = parts.next()?.trim().parse::<i32>().ok()?;
            let amount = parts.next()?.trim().parse::<i32>().ok()?;
            if parts.next().is_some() { return None; }
            Some((start_round, amount))
        })
        .filter(|(start_round, _)| *start_round == round_index)
        .map(|(_, amount)| amount.max(0))
        .sum()
}

pub fn seed_attacker_power_from_cloth(fight: &mut Fight, cloth: &config::cloth_level::ClothLevel) {
    if let Some(attacker) = fight.attacker.as_mut()
        && attacker.power.is_none()
    {
        attacker.power = Some(cloth.initial.max(0));
    }
}

pub fn apply_cloth_power_delta(fight: &mut Fight, cloth: &config::cloth_level::ClothLevel, delta: i32) {
    let Some(attacker) = fight.attacker.as_mut() else { return };
    let current = attacker.power.unwrap_or(cloth.initial.max(0));
    attacker.power = Some((current + delta).clamp(0, cloth.max_power.max(0)));
}

pub fn cloth_power_delta_for_operation(oper: &BeginRoundOper, cloth: &config::cloth_level::ClothLevel) -> i32 {
    match CardOpType::try_from(oper.oper_type.unwrap_or(0)) {
        Ok(CardOpType::MoveCard) | Ok(CardOpType::MoveUniversal) => cloth.r#move.max(0),
        Ok(CardOpType::PlayCard) => cloth.r#use.max(0),
        Ok(CardOpType::SimulateDissolveCard) => cloth.compose.max(0),
        _ => 0,
    }
}
