use sonettobuf::{CardInfo, Fight};

fn next_tier(card: &CardInfo, fight: &Fight) -> Option<i32> {
    let skill_id = card.skill_id?;
    let uid = card.uid.unwrap_or(0);
    let entity = fight
        .attacker
        .as_ref()?
        .entitys
        .iter()
        .find(|e| e.uid.unwrap_or(0) == uid)
        .or_else(|| {
            fight.defender.as_ref()?.entitys.iter().find(|e| e.uid.unwrap_or(0) == uid)
        })?;
    for group in [&entity.skill_group1, &entity.skill_group2] {
        if let Some(pos) = group.iter().position(|&id| id == skill_id) {
            if pos + 1 < group.len() {
                return Some(group[pos + 1]);
            }
        }
    }
    None
}

pub fn apply_card_upgrades(player_deck: &mut Vec<CardInfo>, fight: &Fight) {
    let mut i = 0;
    while i + 1 < player_deck.len() {
        if player_deck[i].skill_id == player_deck[i + 1].skill_id {
            if let Some(next_id) = next_tier(&player_deck[i], fight) {
                player_deck[i].skill_id = Some(next_id);
                player_deck.remove(i + 1);
                continue;
            }
        }
        i += 1;
    }
}
