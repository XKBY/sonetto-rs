use crate::state::battle::{
    ConditionType,
    skill::{cache::resolve_skill_effect_id, condition::parser::parse_condition},
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
    match condition {
        ConditionType::ActiveUseSkill
        | ConditionType::ActiveUseSkillId { .. }
        | ConditionType::CombatNone
        | ConditionType::UseExSkill
        | ConditionType::TeammateUseExSkill
        | ConditionType::BeAttacked
        | ConditionType::HurtNotRestraint
        | ConditionType::HurtRestraint
        | ConditionType::TeammateInjuryCount
        | ConditionType::TeamInjuryCountRound
        | ConditionType::NoActRound
        | ConditionType::BuffIdDel { .. }
        | ConditionType::BloodPool
        | ConditionType::BloodPoolMax { .. }
        | ConditionType::TriggerBullet => true,
        ConditionType::CareerCheck { .. } => include_career_check,
        ConditionType::PerDecrExPoint { .. } => include_per_decr_ex_point,
        ConditionType::EnterFightAnd(conds) | ConditionType::EnterFightOr(conds) => conds
            .iter()
            .any(|c| is_combat_event_condition(c, include_per_decr_ex_point, include_career_check)),
        _ => false,
    }
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
