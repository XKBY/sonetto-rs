use std::collections::HashSet;

use sonettobuf::{CardInfo, FightRound, FightStep};

fn collect_sp_card_adds(step: &FightStep, out: &mut Vec<CardInfo>) {
    for effect in &step.act_effect {
        if effect.effect_type == Some(78) {
            let skill_id = effect.effect_num.unwrap_or(0);
            if skill_id != 0 {
                out.push(CardInfo {
                    uid: Some(0),
                    skill_id: Some(skill_id),
                    card_effect: Some(0),
                    temp_card: Some(true),
                    enchants: vec![],
                    card_type: Some(0),
                    hero_id: Some(0),
                    status: Some(0),
                    target_uid: Some(0),
                    extra_info: None,
                    energy: Some(0),
                    extra_infos: vec![],
                    area_red_or_blue: Some(0),
                    heat_id: Some(0),
                    music_note: None,
                });
            }
        }
        if let Some(inner) = &effect.fight_step {
            collect_sp_card_adds(inner, out);
        }
    }
}

pub fn build_opening_deck(round: &FightRound) -> Vec<CardInfo> {
    let mut cards = round.team_a_cards1.clone();
    let mut sp_cards = Vec::new();
    for step in &round.fight_step {
        collect_sp_card_adds(step, &mut sp_cards);
    }

    let mut seen: HashSet<(i64, i32)> = cards
        .iter()
        .map(|c| (c.uid.unwrap_or(0), c.skill_id.unwrap_or(0)))
        .collect();
    for c in sp_cards {
        let key = (c.uid.unwrap_or(0), c.skill_id.unwrap_or(0));
        if !seen.contains(&key) {
            seen.insert(key);
            cards.push(c);
        }
    }
    cards
}

pub fn apply_opening_deck(round: &mut FightRound) -> Vec<CardInfo> {
    build_opening_deck(round)
}
