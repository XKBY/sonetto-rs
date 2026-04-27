use anyhow::{Context, Result};
use gameserver::state::{BattleSimulator, FightDataMgr};
use serde_json::{Value, json};
use sonettobuf::{
    ActEffect, BeginRoundOper, BuffInfo, CardInfo, Fight, FightExPointInfo, FightRound, FightStep,
    StartDungeonReply, fight_step,
};

use crate::parser::start_dungeon::InitialBuffAddSeed;

pub fn generate_begin_round_reply(
    fight_input: Fight,
    player_deck: Vec<CardInfo>,
    ai_deck: Vec<CardInfo>,
) -> Result<StartDungeonReply> {
    let battle_id = fight_input.battle_id.unwrap_or_default();
    let mut mgr = FightDataMgr::new(fight_input);
    let round = mgr
        .build_initial_round(battle_id, player_deck, ai_deck)
        .context("failed to build initial round")?;

    Ok(StartDungeonReply {
        fight: mgr.pre_fight.clone().or_else(|| Some(mgr.fight().clone())),
        round: Some(round),
    })
}

pub async fn generate_begin_round_sequence(
    fight_input: Fight,
    initial_ex_point_info: Vec<FightExPointInfo>,
    initial_round: Option<FightRound>,
    initial_bloodpool_effects: Vec<(i32, i32, i32)>,
    initial_buff_add_effects: Vec<InitialBuffAddSeed>,
    rounds: Vec<(
        String,
        Vec<CardInfo>,
        Vec<CardInfo>,
        Vec<BeginRoundOper>,
        Vec<FightStep>,
    )>,
) -> Result<Vec<(String, Value)>> {
    let mut mgr = FightDataMgr::new(fight_input);
    let attacker_uids: std::collections::HashSet<i64> = mgr
        .fight()
        .attacker
        .as_ref()
        .map(|a| {
            a.entitys
                .iter()
                .chain(a.sub_entitys.iter())
                .filter_map(|e| e.uid)
                .collect()
        })
        .unwrap_or_default();
    let has_attacker_buff_seed =
        initial_buff_add_effects
            .iter()
            .any(|(buff_id, target_uid, ..)| {
                attacker_uids.contains(target_uid) && is_bootstrap_relevant_buff(*buff_id)
            });
    if !initial_bloodpool_effects.is_empty() || has_attacker_buff_seed {
        if let Some(initial_round) = initial_round.as_ref() {
            mgr.seed_replay_state(initial_round, &initial_ex_point_info)?;
        } else {
            apply_ex_point_seed_to_fight(mgr.fight_mut(), &initial_ex_point_info);
        }
        mgr.seed_replay_buffs_from_effects(&initial_buff_add_effects);
        mgr.seed_replay_bloodtithe_from_effects(&initial_bloodpool_effects);
    } else {
        apply_ex_point_seed_to_fight(mgr.fight_mut(), &initial_ex_point_info);
    }
    // Begin-round replay mode should follow captured round requests directly.
    let mut simulator = BattleSimulator::new(mgr);

    let mut out_rounds: Vec<(String, Value)> = Vec::new();
    for (name, deck, ai_deck, opers, ai_steps) in rounds {
        let ai_override_steps = if should_replay_enemy_steps(&ai_deck, &ai_steps) {
            Some(ai_steps)
        } else {
            None
        };
        let mut generated = simulator
            .process_round(opers.clone(), deck, ai_deck, ai_override_steps)
            .await
            .with_context(|| format!("failed simulating {}", name))?;
        patch_inline_passive_fanout_output(&mut generated);

        let _ = opers; // request metadata intentionally omitted for live-shape compare output.
        out_rounds.push((name, json!({ "round": generated })));
    }

    let _ = simulator.into_data();
    Ok(out_rounds)
}

fn is_bootstrap_relevant_buff(buff_id: i32) -> bool {
    if buff_id <= 0 {
        return false;
    }

    let cfg = config::configs::get();
    let Some(_buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };

    let mut stack = vec![buff_id];
    let mut seen = std::collections::HashSet::new();
    while let Some(current_buff_id) = stack.pop() {
        if current_buff_id <= 0 || !seen.insert(current_buff_id) {
            continue;
        }
        let Some(current) = cfg.skill_buff.iter().find(|b| b.id == current_buff_id) else {
            continue;
        };
        for entry in current.features.split('|') {
            let parts: Vec<&str> = entry.split('#').collect();
            let act_id = parts
                .first()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .unwrap_or(0);
            let act_type = cfg
                .buff_act
                .iter()
                .find(|a| a.id == act_id)
                .map(|a| a.r#type.as_str())
                .unwrap_or("");

            if matches!(
                act_type,
                "BloodPoolTag"
                    | "BloodPoolCountAddExPoint"
                    | "ExPointOverflowBank"
                    | "Raspberry"
                    | "MasterHalo"
                    | "SlaveHalo"
            ) {
                return true;
            }

            if act_type == "SubBuff" {
                for raw in parts.iter().skip(1) {
                    for piece in raw.split(',') {
                        if let Ok(child_buff_id) = piece.trim().parse::<i32>() {
                            stack.push(child_buff_id);
                        }
                    }
                }
            }
        }
    }

    false
}

fn has_duplicate_ai_casts(steps: &[FightStep]) -> bool {
    let mut seen = std::collections::HashSet::new();
    for step in steps {
        let key = (
            step.from_id.unwrap_or(0),
            step.act_id.unwrap_or(0),
            step.to_id.unwrap_or(0),
        );
        if !seen.insert(key) {
            return true;
        }
    }
    false
}

fn should_replay_enemy_steps(ai_deck: &[CardInfo], ai_steps: &[FightStep]) -> bool {
    if ai_steps.is_empty() || has_duplicate_ai_casts(ai_steps) {
        return true;
    }

    let ai_skill_ids: std::collections::HashSet<i32> = ai_deck
        .iter()
        .filter_map(|card| card.skill_id)
        .filter(|skill_id| *skill_id > 0)
        .collect();

    ai_steps.iter().any(|step| {
        let skill_id = step.act_id.unwrap_or(0);
        skill_id > 0 && !ai_skill_ids.contains(&skill_id)
    })
}

fn apply_ex_point_seed_to_fight(fight: &mut Fight, ex_infos: &[FightExPointInfo]) {
    if ex_infos.is_empty() {
        return;
    }

    let mut by_uid = std::collections::HashMap::new();
    for info in ex_infos {
        if let Some(uid) = info.uid {
            by_uid.insert(uid, info);
        }
    }

    for team in [&mut fight.attacker, &mut fight.defender] {
        if let Some(side) = team {
            for entity in side.entitys.iter_mut().chain(side.sub_entitys.iter_mut()) {
                let uid = entity.uid.unwrap_or(0);
                if let Some(info) = by_uid.get(&uid) {
                    entity.ex_point = Some(info.ex_point.unwrap_or(0));
                    if let Some(current_hp) = info.current_hp {
                        entity.current_hp = Some(current_hp);
                    }
                }
            }
        }
    }
}

fn patch_inline_passive_fanout_output(round: &mut FightRound) {
    let tuesday_uid = round.fight_step.iter().find_map(|step| {
        let skill_id = step.act_id.unwrap_or(0);
        (matches!(skill_id, 30980111 | 30980121 | 30980131)).then(|| step.from_id.unwrap_or(0))
    });

    for step in &mut round.fight_step {
        patch_host_step_output(step, tuesday_uid);
    }
}

fn patch_host_step_output(step: &mut FightStep, tuesday_uid: Option<i64>) {
    if step.act_type != Some(fight_step::ActType::Skill as i32) {
        return;
    }

    let had_duality = rewrite_sotheby_duality_wrapper_output(step);
    lift_foreign_origin_nested_wrappers(step);
    let had_tuesday_wrappers = strip_tuesday_wrappers(step);
    if had_duality || had_tuesday_wrappers {
        inject_tuesday_wrapper_output(step, tuesday_uid);
    }
    normalize_skill_wrapper_order(step);
}

fn wrapper_act_id(effect: &ActEffect) -> Option<i32> {
    (effect.effect_type == Some(162))
        .then(|| effect.fight_step.as_ref().and_then(|s| s.act_id))
        .flatten()
}

fn wrapper_from_id(effect: &ActEffect) -> i64 {
    effect
        .fight_step
        .as_ref()
        .and_then(|s| s.from_id)
        .unwrap_or(0)
}

fn is_damage_effect_type(effect_type: i32) -> bool {
    matches!(effect_type, 2 | 3)
}

fn is_poison_family_buff(buff_id: i32) -> bool {
    if buff_id <= 0 {
        return false;
    }
    config::configs::get()
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| b.type_id == 6003)
        .unwrap_or(false)
}

fn collect_poisoned_enemy_targets(step: &FightStep) -> Vec<i64> {
    let caster_uid = step.from_id.unwrap_or(0);
    let mut out = Vec::new();
    for effect in &step.act_effect {
        if effect.effect_type != Some(5) {
            continue;
        }
        let Some(buff_id) = effect.buff.as_ref().and_then(|b| b.buff_id) else {
            continue;
        };
        let target_uid = effect.target_id.unwrap_or(0);
        if target_uid == 0
            || target_uid.signum() == caster_uid.signum()
            || !is_poison_family_buff(buff_id)
        {
            continue;
        }
        if !out.contains(&target_uid) {
            out.push(target_uid);
        }
    }
    out
}

fn collect_impacted_enemy_targets(step: &FightStep) -> Vec<i64> {
    collect_poisoned_enemy_targets(step)
}

fn poison_marker(target_uid: i64) -> ActEffect {
    ActEffect {
        effect_type: Some(213),
        target_id: Some(target_uid),
        effect_num: Some(0),
        ..Default::default()
    }
}

fn build_effect_wrapper(
    from_uid: i64,
    to_uid: i64,
    act_id: i32,
    effects: Vec<ActEffect>,
) -> ActEffect {
    ActEffect {
        effect_type: Some(162),
        target_id: Some(0),
        effect_num: Some(0),
        fight_step: Some(FightStep {
            act_type: Some(fight_step::ActType::Effect as i32),
            from_id: Some(from_uid),
            to_id: Some(to_uid),
            act_id: Some(act_id),
            act_effect: effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }),
        ..Default::default()
    }
}

fn build_buff_add(target_uid: i64, from_uid: i64, buff_id: i32) -> ActEffect {
    ActEffect {
        effect_type: Some(5),
        target_id: Some(target_uid),
        effect_num: Some(buff_id),
        buff: Some(BuffInfo {
            buff_id: Some(buff_id),
            duration: Some(0),
            uid: Some(0),
            ex_info: Some(0),
            from_uid: Some(from_uid),
            count: Some(0),
            act_common_params: Some(String::new()),
            layer: Some(0),
            r#type: Some(0),
            act_info: vec![],
        }),
        ..Default::default()
    }
}

fn rewrite_sotheby_duality_wrapper_output(step: &mut FightStep) -> bool {
    let Some(wrapper_idx) = step
        .act_effect
        .iter()
        .position(|effect| wrapper_act_id(effect) == Some(30090146))
    else {
        return false;
    };

    let caster_uid = step.from_id.unwrap_or(0);
    let poison_targets = collect_poisoned_enemy_targets(step);
    if caster_uid == 0 || poison_targets.is_empty() {
        return false;
    }

    let removed = step.act_effect.remove(wrapper_idx);
    let removed_buff = removed
        .fight_step
        .as_ref()
        .and_then(|s| s.act_effect.first())
        .and_then(|e| e.buff.as_ref())
        .cloned();

    let mut proc_effects = Vec::new();
    for target_uid in poison_targets {
        proc_effects.push(build_buff_add(target_uid, caster_uid, 300901412));
        proc_effects.push(poison_marker(target_uid));
    }

    let mut replacements = vec![build_effect_wrapper(
        caster_uid,
        caster_uid,
        30091120,
        proc_effects,
    )];

    if let Some(buff) = removed_buff {
        let buff_uid = buff.uid.unwrap_or(0);
        if buff_uid != 0 {
            replacements.push(build_effect_wrapper(
                caster_uid,
                caster_uid,
                30091120,
                vec![ActEffect {
                    effect_type: Some(6),
                    target_id: Some(caster_uid),
                    effect_num: Some(0),
                    buff: Some(BuffInfo {
                        buff_id: Some(buff.buff_id.unwrap_or(30091120)),
                        duration: Some(0),
                        uid: Some(buff_uid),
                        ex_info: Some(0),
                        from_uid: Some(buff.from_uid.unwrap_or(caster_uid)),
                        count: Some(0),
                        act_common_params: Some(String::new()),
                        layer: Some(0),
                        r#type: Some(0),
                        act_info: vec![],
                    }),
                    ..Default::default()
                }],
            ));
        }
    }

    step.act_effect
        .splice(wrapper_idx..wrapper_idx, replacements);
    true
}

fn lift_foreign_origin_nested_wrappers(step: &mut FightStep) {
    let host_from = step.from_id.unwrap_or(0);
    let mut lifted = Vec::new();
    for effect in &mut step.act_effect {
        let Some(child) = effect.fight_step.as_mut() else {
            continue;
        };
        if child.act_type != Some(fight_step::ActType::Skill as i32) {
            continue;
        }
        let child_from = child.from_id.unwrap_or(host_from);
        let mut kept = Vec::with_capacity(child.act_effect.len());
        for nested in child.act_effect.drain(..) {
            if nested.effect_type == Some(162) {
                let nested_from = wrapper_from_id(&nested);
                if nested_from != 0 && nested_from != child_from {
                    lifted.push(nested);
                    continue;
                }
            }
            kept.push(nested);
        }
        child.act_effect = kept;
    }
    step.act_effect.extend(lifted);
}

fn inject_tuesday_wrapper_output(step: &mut FightStep, tuesday_uid: Option<i64>) {
    if step
        .act_effect
        .iter()
        .any(|effect| wrapper_act_id(effect) == Some(30980142))
    {
        return;
    }

    let Some(holder_uid) = tuesday_uid else {
        return;
    };
    let caster_uid = step.from_id.unwrap_or(0);
    if holder_uid == 0 || caster_uid == 0 || holder_uid == caster_uid {
        return;
    }

    let impacted_targets = collect_impacted_enemy_targets(step);
    if impacted_targets.is_empty() {
        return;
    }

    let mut inner_effects = Vec::new();
    for target_uid in impacted_targets {
        inner_effects.push(build_buff_add(target_uid, caster_uid, 30980145));
        inner_effects.push(poison_marker(target_uid));
    }
    step.act_effect.push(build_effect_wrapper(
        holder_uid,
        caster_uid,
        30980142,
        inner_effects,
    ));
}

fn strip_tuesday_wrappers(step: &mut FightStep) -> bool {
    let before = step.act_effect.len();
    step.act_effect
        .retain(|effect| wrapper_act_id(effect) != Some(30980142));
    before != step.act_effect.len()
}

fn normalize_skill_wrapper_order(step: &mut FightStep) {
    if step.act_type != Some(fight_step::ActType::Skill as i32) || step.act_effect.len() < 3 {
        return;
    }

    let mut idx = 0usize;
    while idx < step.act_effect.len()
        && step.act_effect[idx]
            .effect_type
            .is_some_and(is_damage_effect_type)
    {
        idx += 1;
    }
    if idx == 0 || idx >= step.act_effect.len() {
        return;
    }

    let prefix = step.act_effect[..idx].to_vec();
    let mut wrappers = Vec::new();
    let mut tail = Vec::new();
    for effect in step.act_effect[idx..].iter().cloned() {
        if effect.effect_type == Some(162) {
            wrappers.push(effect);
        } else {
            tail.push(effect);
        }
    }
    if wrappers.is_empty() {
        return;
    }

    wrappers.sort_by_key(|effect| {
        let act_type = effect
            .fight_step
            .as_ref()
            .and_then(|s| s.act_type)
            .unwrap_or(0);
        if act_type == fight_step::ActType::Effect as i32 {
            0
        } else {
            1
        }
    });

    let mut reordered = prefix;
    reordered.extend(wrappers);
    reordered.extend(tail);
    step.act_effect = reordered;
}
