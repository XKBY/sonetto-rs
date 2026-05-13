use rand::Rng;
use sonettobuf::CardInfo;
use std::collections::HashMap;

fn skill_key(card: &CardInfo) -> i32 {
    card.skill_id.unwrap_or(0)
}

fn separate_adjacent_same_skill(cards: &mut [CardInfo]) {
    if cards.len() < 2 {
        return;
    }

    for i in 1..cards.len() {
        if skill_key(&cards[i]) != skill_key(&cards[i - 1]) {
            continue;
        }

        if let Some(j) =
            ((i + 1)..cards.len()).find(|&k| skill_key(&cards[k]) != skill_key(&cards[i - 1]))
        {
            cards.swap(i, j);
        }
    }
}

pub fn draw_deck_guaranteed_by_uid_with_rng<R: Rng + ?Sized>(
    cards: &[CardInfo],
    required_uids: &[i64],
    count: usize,
    rng: &mut R,
) -> Vec<CardInfo> {
    if cards.is_empty() || count == 0 {
        return vec![];
    }

    let mut by_uid: HashMap<i64, Vec<&CardInfo>> = HashMap::new();
    for card in cards {
        by_uid.entry(card.uid.unwrap_or(0)).or_default().push(card);
    }

    let mut out: Vec<CardInfo> = Vec::with_capacity(count);

    // First pass: guarantee at least one card for each required hero uid (if available).
    for &uid in required_uids {
        if out.len() >= count {
            break;
        }
        if let Some(options) = by_uid.get(&uid)
            && !options.is_empty()
        {
            let prev_key = out.last().map(skill_key);
            let non_touching: Vec<&CardInfo> = match prev_key {
                Some(prev) => options
                    .iter()
                    .copied()
                    .filter(|c| skill_key(c) != prev)
                    .collect(),
                None => options.clone(),
            };

            let picked = if non_touching.is_empty() {
                let idx = rng.gen_range(0..options.len());
                options[idx]
            } else {
                let idx = rng.gen_range(0..non_touching.len());
                non_touching[idx]
            };
            out.push(picked.clone());
        }
    }

    // Fill the remaining hand from the full pool.
    while out.len() < count {
        let prev_key = out.last().map(skill_key);
        let non_touching: Vec<&CardInfo> = match prev_key {
            Some(prev) => cards.iter().filter(|c| skill_key(c) != prev).collect(),
            None => cards.iter().collect(),
        };

        let picked = if non_touching.is_empty() {
            let idx = rng.gen_range(0..cards.len());
            &cards[idx]
        } else {
            let idx = rng.gen_range(0..non_touching.len());
            non_touching[idx]
        };

        out.push(picked.clone());
    }

    separate_adjacent_same_skill(&mut out);

    // Game rule: For odd-sized hands (5, 7), first card must equal last card
    if count % 2 == 1 && out.len() == count && count > 0 {
        out[count - 1] = out[0].clone();
    }

    out
}
