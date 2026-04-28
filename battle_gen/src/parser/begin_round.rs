use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::Value;
use sonettobuf::card_info::{CardStatus, CardType};
use sonettobuf::{BeginRoundOper, CardInfo, FightStep, fight_step::ActType};

fn read_json(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

pub fn load_begin_round_captures(dir: &Path) -> Result<Vec<(PathBuf, Value, Option<Value>)>> {
    let files: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("failed to read begin-round dir {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| {
                    n.starts_with("begin_round_")
                        && n.ends_with(".json")
                        && !n.ends_with("_request.json")
                })
                .unwrap_or(false)
        })
        .collect();

    let mut files = files;
    files.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("begin_round_"))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });

    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let raw = read_json(&path)?;
        let round_v: Value = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse begin-round JSON {}", path.display()))?;

        let request_path = {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            path.with_file_name(format!("{stem}_request.json"))
        };
        let request_v = if request_path.exists() {
            let request_raw = read_json(&request_path)?;
            Some(
                serde_json::from_str::<Value>(&request_raw).with_context(|| {
                    format!(
                        "failed to parse begin-round request {}",
                        request_path.display()
                    )
                })?,
            )
        } else {
            None
        };

        out.push((path, round_v, request_v));
    }
    Ok(out)
}

pub fn extract_begin_round_inputs(
    capture: &Value,
    request: Option<&Value>,
) -> Result<(
    Vec<CardInfo>,
    Vec<CardInfo>,
    Vec<BeginRoundOper>,
    Vec<FightStep>,
    Vec<CardInfo>,
    Vec<bool>,
)> {
    let round = capture.get("round").unwrap_or(capture);
    let steps = round
        .get("fightStep")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut selected_cards_v = Value::Array(vec![]);
    let mut remaining_cards_v = Value::Array(vec![]);
    for step in &steps {
        if act_type_is_skill(step) {
            continue;
        }
        if let Some(effects) = step.get("actEffect").and_then(Value::as_array) {
            for e in effects {
                let effect_type = e.get("effectType").and_then(Value::as_i64).unwrap_or(0);
                if effect_type == 159 && selected_cards_v.as_array().is_none_or(|a| a.is_empty()) {
                    selected_cards_v = e
                        .get("cardInfoList")
                        .cloned()
                        .unwrap_or(Value::Array(vec![]));
                } else if effect_type == 154
                    && remaining_cards_v.as_array().is_none_or(|a| a.is_empty())
                {
                    remaining_cards_v = e
                        .get("cardInfoList")
                        .cloned()
                        .unwrap_or(Value::Array(vec![]));
                }
            }
        }
    }

    normalize_card_info_enums(&mut selected_cards_v);
    normalize_card_info_enums(&mut remaining_cards_v);

    let selected_cards: Vec<CardInfo> = serde_json::from_value(selected_cards_v)
        .context("failed to parse selected cards (effect 159)")?;
    let remaining_cards: Vec<CardInfo> = serde_json::from_value(remaining_cards_v)
        .context("failed to parse remaining cards (effect 154)")?;

    let mut deck = selected_cards.clone();
    deck.extend(remaining_cards.clone());

    let mut ai_cards_v = round
        .get("aiUseCards")
        .or_else(|| round.get("ai_use_cards"))
        .cloned()
        .unwrap_or(Value::Array(vec![]));
    normalize_card_info_enums(&mut ai_cards_v);
    let ai_cards: Vec<CardInfo> =
        serde_json::from_value(ai_cards_v).context("failed to parse aiUseCards")?;

    let opers = if let Some(req) = request {
        let opers_v = req.get("opers").cloned().unwrap_or(Value::Array(vec![]));
        match serde_json::from_value::<Vec<BeginRoundOper>>(opers_v.clone()) {
            Ok(v) => v,
            Err(_) => {
                // Backward-compatible fallback if a payload uses snake_case.
                let mut fallback_v = opers_v;
                normalize_begin_round_opers_enums(&mut fallback_v);
                serde_json::from_value::<Vec<BeginRoundOper>>(fallback_v)
                    .context("failed to parse request.opers")?
            }
        }
    } else {
        // Fallback for older captures without explicit request file.
        let mut opers = Vec::new();
        let mut working = deck.clone();
        for step in &steps {
            if !act_type_is_skill(&step) {
                continue;
            }
            let from_id = value_as_i64(step.get("fromId")).unwrap_or(0);
            if from_id < 0 {
                break;
            }
            let to_id = value_as_i64(step.get("toId")).unwrap_or(0);
            let skill_id = value_as_i64(step.get("actId")).unwrap_or(0) as i32;
            if skill_id == 0 {
                continue;
            }

            let idx_opt = if from_id > 0 {
                working.iter().position(|c| {
                    c.uid.unwrap_or(0) == from_id && c.skill_id.unwrap_or(0) == skill_id
                })
            } else {
                working
                    .iter()
                    .position(|c| c.skill_id.unwrap_or(0) == skill_id && c.uid.unwrap_or(0) == 0)
                    .or_else(|| {
                        working
                            .iter()
                            .position(|c| c.skill_id.unwrap_or(0) == skill_id)
                    })
            };

            let Some(idx) = idx_opt else { continue };
            working.remove(idx);

            opers.push(BeginRoundOper {
                oper_type: Some(2),
                param1: Some((idx as i32) + 1),
                to_id: Some(to_id),
                ..Default::default()
            });
        }
        opers
    };

    if !selected_cards.is_empty() && !remaining_cards.is_empty() && !opers.is_empty() {
        if let Some(rebuilt) = rebuild_pre_pick_deck(&selected_cards, &remaining_cards, &opers) {
            deck = rebuilt;
        }
    }

    let top_attacker_skills: Vec<(i32, i64)> = round
        .get("fightStep")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| {
                    if !act_type_is_skill(step) {
                        return None;
                    }
                    let from_id = value_as_i64(step.get("fromId")).unwrap_or(0);
                    if from_id < 0 {
                        return None;
                    }
                    let act_id = value_as_i64(step.get("actId")).unwrap_or(0) as i32;
                    if act_id == 0 {
                        return None;
                    }
                    Some((act_id, from_id))
                })
                .collect()
        })
        .unwrap_or_default();

    let mut replay_silent_ops: Vec<bool> = Vec::with_capacity(selected_cards.len());
    let mut top_idx = 0usize;
    for sel in &selected_cards {
        let sel_skill = sel.skill_id.unwrap_or(0) as i32;
        let sel_uid = sel.uid.unwrap_or(0);
        let matched = top_idx < top_attacker_skills.len()
            && top_attacker_skills[top_idx].0 == sel_skill
            && top_attacker_skills[top_idx].1 == sel_uid;
        if matched {
            replay_silent_ops.push(false);
            top_idx += 1;
        } else {
            replay_silent_ops.push(true);
        }
    }

    let mut enemy_steps_v = Value::Array(
        steps
            .into_iter()
            .filter(|step| {
                act_type_is_skill(step) && value_as_i64(step.get("fromId")).unwrap_or(0) < 0
            })
            .collect(),
    );
    normalize_fight_step_array_enums(&mut enemy_steps_v);
    let enemy_steps: Vec<FightStep> =
        serde_json::from_value(enemy_steps_v).context("failed to parse enemy fightStep replay")?;

    Ok((
        deck,
        ai_cards,
        opers,
        enemy_steps,
        selected_cards,
        replay_silent_ops,
    ))
}

fn act_type_is_skill(step: &Value) -> bool {
    match step.get("actType") {
        Some(Value::String(s)) => s.eq_ignore_ascii_case("SKILL"),
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
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

fn normalize_fight_step_array_enums(steps_value: &mut Value) {
    let Some(steps) = steps_value.as_array_mut() else {
        return;
    };
    for step in steps {
        normalize_fight_step_enums(step);
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

fn normalize_begin_round_opers_enums(opers_value: &mut Value) {
    let Some(opers) = opers_value.as_array_mut() else {
        return;
    };

    for oper in opers {
        let Some(obj) = oper.as_object_mut() else {
            continue;
        };

        // Support snake_case payloads by remapping to camelCase.
        remap_key(obj, "oper_type", "operType");
        remap_key(obj, "to_id", "toId");
    }
}

fn value_as_i64(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn remap_key(obj: &mut serde_json::Map<String, Value>, from: &str, to: &str) {
    if obj.contains_key(to) {
        return;
    }
    if let Some(v) = obj.remove(from) {
        obj.insert(to.to_string(), v);
    }
}

fn rebuild_pre_pick_deck(
    selected_cards: &[CardInfo],
    remaining_cards: &[CardInfo],
    opers: &[BeginRoundOper],
) -> Option<Vec<CardInfo>> {
    let play_opers: Vec<&BeginRoundOper> = opers
        .iter()
        .filter(|o| o.oper_type.unwrap_or(0) == 2)
        .collect();

    if play_opers.is_empty() || selected_cards.len() < play_opers.len() {
        return None;
    }

    let mut deck = remaining_cards.to_vec();
    let selected_tail = &selected_cards[selected_cards.len() - play_opers.len()..];

    for (oper, selected) in play_opers.iter().zip(selected_tail.iter()).rev() {
        let idx = oper.param1.unwrap_or(1).saturating_sub(1) as usize;
        if idx > deck.len() {
            return None;
        }
        deck.insert(idx, selected.clone());
    }

    Some(deck)
}
