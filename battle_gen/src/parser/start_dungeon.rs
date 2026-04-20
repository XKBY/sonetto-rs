use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sonettobuf::card_info::{CardStatus, CardType};
use sonettobuf::fight::FightActType;
use sonettobuf::fight_step::ActType;
use sonettobuf::{CardInfo, Fight, FightExPointInfo, FightRound};

pub type InitialBuffAddSeed = (i32, i64, i64, i32, i32, i64, i32);

fn read_json(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

/// Parse the fight object for start-dungeon generation input.
/// Supported layouts:
/// - raw Fight object
/// - wrapper object containing `{ "fight": ... }`
pub fn load_fight(path: &Path) -> Result<Fight> {
    let raw = read_json(path)?;

    let mut v: Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))?;

    if let Some(fight_v) = v.get_mut("fight") {
        normalize_fight_enums(fight_v);
        return serde_json::from_value::<Fight>(fight_v.clone())
            .with_context(|| format!("failed to deserialize `fight` in {}", path.display()));
    }

    normalize_fight_enums(&mut v);
    if let Ok(fight) = serde_json::from_value::<Fight>(v.clone()) {
        return Ok(fight);
    }

    bail!(
        "{} must be either a raw Fight object or {{\"fight\": ...}}",
        path.display()
    );
}

/// Parse cards from a start-dungeon payload.
pub fn load_cards_used_decks(path: &Path) -> (Vec<CardInfo>, Vec<CardInfo>) {
    if !path.exists() {
        return (vec![], vec![]);
    }

    let raw = match read_json(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: cards-used read failed ({}), using empty decks",
                e
            );
            return (vec![], vec![]);
        }
    };
    let v: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: cards-used JSON parse failed for {} ({e}), using empty decks",
            path.display()
        );
        Value::Object(Default::default())
    });
    let cards_used = v
        .get("cardsUsed")
        .or_else(|| v.get("cards_used"))
        .or_else(|| v.get("round"))
        .cloned()
        .unwrap_or_else(|| v.clone());

    let team_a_cards = cards_used
        .get("teamACards1")
        .or_else(|| cards_used.get("team_a_cards1"))
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    let ai_use_cards = cards_used
        .get("aiUseCards")
        .or_else(|| cards_used.get("ai_use_cards"))
        .cloned()
        .unwrap_or(Value::Array(vec![]));

    let mut team_a_cards = team_a_cards;
    normalize_card_info_enums(&mut team_a_cards);
    let mut ai_use_cards = ai_use_cards;
    normalize_card_info_enums(&mut ai_use_cards);

    let team_a_cards1: Vec<CardInfo> = match serde_json::from_value(team_a_cards) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: failed to deserialize teamACards1 from {} ({e}), using empty deck",
                path.display()
            );
            vec![]
        }
    };
    let ai_use_cards: Vec<CardInfo> = match serde_json::from_value(ai_use_cards) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: failed to deserialize aiUseCards from {} ({e}), using empty deck",
                path.display()
            );
            vec![]
        }
    };

    (team_a_cards1, ai_use_cards)
}

pub fn load_initial_ex_point_info(path: &Path) -> Vec<FightExPointInfo> {
    if !path.exists() {
        return vec![];
    }

    let raw = match read_json(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: ex-point seed read failed ({}), using empty exPointInfo",
                e
            );
            return vec![];
        }
    };

    let v: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: ex-point seed JSON parse failed for {} ({e}), using empty exPointInfo",
            path.display()
        );
        Value::Object(Default::default())
    });

    let ex_v = v
        .get("round")
        .and_then(|r| r.get("exPointInfo").or_else(|| r.get("ex_point_info")))
        .cloned()
        .unwrap_or(Value::Array(vec![]));

    serde_json::from_value::<Vec<FightExPointInfo>>(ex_v).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: failed to deserialize exPointInfo from {} ({e}), using empty seed",
            path.display()
        );
        vec![]
    })
}

pub fn load_initial_round(path: &Path) -> Option<FightRound> {
    if !path.exists() {
        return None;
    }

    let raw = match read_json(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: initial-round read failed ({}), skipping round seed",
                e
            );
            return None;
        }
    };

    let v: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: initial-round JSON parse failed for {} ({e}), skipping round seed",
            path.display()
        );
        Value::Object(Default::default())
    });

    let mut round_v = v.get("round").cloned()?;
    normalize_round_enums(&mut round_v);
    serde_json::from_value::<FightRound>(round_v).map_err(|e| {
        eprintln!(
            "battle_gen: failed to deserialize initial round from {} ({e}), skipping round seed",
            path.display()
        );
        e
    }).ok()
}

pub fn load_initial_bloodpool_effects(path: &Path) -> Vec<(i32, i32, i32)> {
    if !path.exists() {
        return vec![];
    }

    let raw = match read_json(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: initial bloodpool read failed ({}), skipping bloodpool seed",
                e
            );
            return vec![];
        }
    };

    let v: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: initial bloodpool JSON parse failed for {} ({e}), skipping bloodpool seed",
            path.display()
        );
        Value::Object(Default::default())
    });

    let Some(steps) = v
        .get("round")
        .and_then(|r| r.get("fightStep"))
        .and_then(|s| s.as_array())
    else {
        return vec![];
    };

    let mut out = Vec::new();
    let mut stack: Vec<&Value> = steps.iter().collect();
    while let Some(step) = stack.pop() {
        let Some(effects) = step.get("actEffect").and_then(|a| a.as_array()) else {
            continue;
        };
        for effect in effects {
            let effect_type = json_i32(effect.get("effectType")).unwrap_or(0);
            if (333..=335).contains(&effect_type) {
                out.push((
                    effect_type,
                    json_i32(effect.get("effectNum"))
                        .or_else(|| json_i32(effect.get("teamType")))
                        .unwrap_or(1),
                    json_i32(effect.get("effectNum1")).unwrap_or(0),
                ));
            }
            if let Some(nested) = effect.get("fightStep") {
                stack.push(nested);
            }
        }
    }

    out.reverse();
    out
}

pub fn load_initial_buff_add_effects(path: &Path) -> Vec<InitialBuffAddSeed> {
    if !path.exists() {
        return vec![];
    }

    let raw = match read_json(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "battle_gen: initial buff seed read failed ({}), skipping buff seed",
                e
            );
            return vec![];
        }
    };

    let v: Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!(
            "battle_gen: initial buff seed JSON parse failed for {} ({e}), skipping buff seed",
            path.display()
        );
        Value::Object(Default::default())
    });

    let Some(steps) = v
        .get("round")
        .and_then(|r| r.get("fightStep"))
        .and_then(|s| s.as_array())
    else {
        return vec![];
    };

    let mut out = Vec::new();
    let mut stack: Vec<&Value> = steps.iter().collect();
    while let Some(step) = stack.pop() {
        let Some(effects) = step.get("actEffect").and_then(|a| a.as_array()) else {
            continue;
        };
        for effect in effects {
            if json_i32(effect.get("effectType")) == Some(5)
                && let Some(buff) = effect.get("buff")
            {
                let buff_id = json_i32(buff.get("buffId")).unwrap_or(0);
                let target_uid = json_i64(effect.get("targetId")).unwrap_or(0);
                let from_uid = json_i64(buff.get("fromUid")).unwrap_or(0);
                let count = json_i32(buff.get("count")).unwrap_or(0);
                let layer = json_i32(buff.get("layer")).unwrap_or(0);
                let buff_uid = json_i64(buff.get("uid")).unwrap_or(0);
                let duration = json_i32(buff.get("duration")).unwrap_or(0);

                if buff_id > 0 && target_uid != 0 && buff_uid > 0 {
                    out.push((
                        buff_id, target_uid, from_uid, count, layer, buff_uid, duration,
                    ));
                }
            }
            if let Some(nested) = effect.get("fightStep") {
                stack.push(nested);
            }
        }
    }

    out.reverse();
    out
}

fn json_i32(v: Option<&Value>) -> Option<i32> {
    let value = v?;
    if let Some(n) = value.as_i64() {
        return i32::try_from(n).ok();
    }
    value.as_str().and_then(|s| s.parse::<i32>().ok())
}

fn json_i64(v: Option<&Value>) -> Option<i64> {
    let value = v?;
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    value.as_str().and_then(|s| s.parse::<i64>().ok())
}

fn normalize_fight_enums(fight_value: &mut Value) {
    let Some(obj) = fight_value.as_object_mut() else {
        return;
    };

    if let Some(fight_act_type_v) = obj.get_mut("fightActType")
        && let Some(name) = fight_act_type_v.as_str()
        && let Some(kind) = FightActType::from_str_name(name)
    {
        *fight_act_type_v = Value::from(kind as i32);
    }
}

fn normalize_round_enums(round_value: &mut Value) {
    let Some(obj) = round_value.as_object_mut() else {
        return;
    };

    if let Some(steps_v) = obj.get_mut("fightStep")
        && let Some(steps) = steps_v.as_array_mut()
    {
        for step in steps {
            normalize_fight_step_enums(step);
        }
    }

    for key in [
        "aiUseCards",
        "teamACards1",
        "teamACards2",
        "beforeCards1",
        "beforeCards2",
    ] {
        if let Some(cards_v) = obj.get_mut(key) {
            normalize_card_info_enums(cards_v);
        }
    }
}

fn normalize_fight_step_enums(step_value: &mut Value) {
    let Some(obj) = step_value.as_object_mut() else {
        return;
    };

    if let Some(act_type_v) = obj.get_mut("actType")
        && let Some(name) = act_type_v.as_str()
        && let Some(kind) = ActType::from_str_name(name)
    {
        *act_type_v = Value::from(kind as i32);
    }

    if let Some(effects_v) = obj.get_mut("actEffect")
        && let Some(effects) = effects_v.as_array_mut()
    {
        for effect in effects {
            let Some(effect_obj) = effect.as_object_mut() else {
                continue;
            };
            if let Some(nested_step_v) = effect_obj.get_mut("fightStep") {
                normalize_fight_step_enums(nested_step_v);
            }
            if let Some(cards_v) = effect_obj.get_mut("cardInfoList") {
                normalize_card_info_enums(cards_v);
            }
        }
    }
}

fn normalize_card_info_enums(cards_value: &mut Value) {
    let Some(cards) = cards_value.as_array_mut() else {
        return;
    };
    for card in cards {
        let Some(obj) = card.as_object_mut() else {
            continue;
        };

        if let Some(card_type_v) = obj.get_mut("cardType")
            && let Some(name) = card_type_v.as_str()
            && let Some(card_type) = CardType::from_str_name(name)
        {
            *card_type_v = Value::from(card_type as i32);
        }

        if let Some(status_v) = obj.get_mut("status")
            && let Some(name) = status_v.as_str()
            && let Some(status) = CardStatus::from_str_name(name)
        {
            *status_v = Value::from(status as i32);
        }

        let uid = obj.get("uid").and_then(Value::as_i64).unwrap_or(0);
        let skill_id = obj.get("skillId").and_then(Value::as_i64).unwrap_or(0);
        if uid == 0 && skill_id > 0 {
            obj.insert("tempCard".to_string(), Value::Bool(true));
        }
    }
}
