use std::collections::HashSet;

use sonettobuf::{CardInfo, FightRound, FightStep};

fn collect_sp_card_adds(step: &FightStep, out: &mut Vec<CardInfo>) {
    for effect in &step.act_effect {
        if effect.effect_type == Some(78) {
            let skill_id = effect.effect_num.unwrap_or(0);
            if skill_id != 0 {
                out.push(CardInfo {
                    // Live sends injected temp cards with neutral identity.
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
    // Do NOT write `round.team_a_cards1` here.
    // Opening visuals should add temp/special cards via fight-step effects (78/141) exactly once.
    // This helper only returns the server-side authoritative deck for post-text sync
    // (`current_deck` / `CardInfoPush.card_group`), to avoid double-add + snapback.
    build_opening_deck(round)
}
