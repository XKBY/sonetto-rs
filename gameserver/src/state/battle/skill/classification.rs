use crate::state::battle::{
    ConditionType,
    skill::{
        cache::resolve_skill_effect_id,
        condition::{self, parser::parse_condition},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatPassiveScanMode {
    RoundSweep,
    TriggerPass,
}

impl CombatPassiveScanMode {
    fn allow_per_decr_ex_point(self) -> bool {
        matches!(self, Self::TriggerPass)
    }

    fn allow_career_check(self) -> bool {
        matches!(self, Self::TriggerPass)
    }

    fn stop_at_first_blank_condition(self) -> bool {
        matches!(self, Self::TriggerPass)
    }
}

pub fn has_combat_reactive_condition(skill_id: i32, mode: CombatPassiveScanMode) -> bool {
    if skill_id <= 0 {
        return false;
    }

    if matches!(mode, CombatPassiveScanMode::TriggerPass)
        && is_damage_reactive_extra_skill(skill_id)
    {
        return true;
    }

    for raw in collect_skill_condition_strings(skill_id, mode.stop_at_first_blank_condition()) {
        let (condition, _) = parse_condition(&raw);
        if is_combat_event_condition(
            &condition,
            mode.allow_per_decr_ex_point(),
            mode.allow_career_check(),
        ) {
            return true;
        }
    }

    false
}

/// True when any condition on the skill references `TeammateInjuryCount` or
/// `TeammateInjuryCountNotReset`. Used by the combat passives pass to replay the
/// skill once per distinct teammate injury (LIVE fires these per-event, not per-batch).
pub fn has_injury_reactive_condition(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    for raw in collect_skill_condition_strings(skill_id, true) {
        let (condition, _) = parse_condition(&raw);
        if condition::is_reactive_passive_condition(
            &condition,
            condition::ReactivePassiveConditionOptions {
                include_teammate_injury_count: true,
                ..Default::default()
            },
        ) {
            return true;
        }
    }
    false
}

pub fn has_be_attacked_reactive_condition(skill_id: i32) -> bool {
    if skill_id <= 0 {
        return false;
    }
    for raw in collect_skill_condition_strings(skill_id, true) {
        let (condition, _) = parse_condition(&raw);
        if condition::is_reactive_passive_condition(
            &condition,
            condition::ReactivePassiveConditionOptions {
                include_be_attacked: true,
                ..Default::default()
            },
        ) {
            return true;
        }
    }
    false
}

fn is_damage_reactive_extra_skill(skill_id: i32) -> bool {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    cfg.skill_effect
        .iter()
        .find(|s| s.id == effect_id)
        .map(|s| s.is_extra > 0 && s.damage_rate > 0)
        .unwrap_or(false)
}

pub fn is_combat_event_condition(
    condition: &ConditionType,
    include_per_decr_ex_point: bool,
    include_career_check: bool,
) -> bool {
    condition::is_combat_event_condition(
        condition,
        condition::CombatEventConditionOptions {
            include_per_decr_ex_point,
            include_career_check,
            ..Default::default()
        },
    )
}

fn collect_skill_condition_strings(skill_id: i32, stop_on_first_empty: bool) -> Vec<String> {
    let cfg = config::configs::get();
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = cfg.skill_effect.iter().find(|s| s.id == effect_id) else {
        return Vec::new();
    };

    let raw_conditions = [
        skill.condition1.as_str(),
        skill.condition2.as_str(),
        skill.condition3.as_str(),
        skill.condition4.as_str(),
        skill.condition5.as_str(),
        skill.condition6.as_str(),
        skill.condition7.as_str(),
        skill.condition8.as_str(),
        skill.condition9.as_str(),
        skill.condition10.as_str(),
    ];

    let mut out = Vec::new();
    for raw in raw_conditions {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            if stop_on_first_empty {
                break;
            }
            continue;
        }
        out.push(trimmed.to_string());
    }

    out
}
