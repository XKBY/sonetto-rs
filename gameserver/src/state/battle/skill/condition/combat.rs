// All conditions in this file are combat-only and always return false at battle start.
// Event-driven conditions (BeAttacked, ActiveUseSkill, etc.) return true in combat context
// because skill_should_fire pre-filters by event before check_condition is ever called.
use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;

pub(super) struct Combat;

impl Condition for Combat {
    fn parse(&self, parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        match cond_type {
            "UseExSkill" => Some(ConditionType::UseExSkill),
            "UseSkillId" => Some(ConditionType::UseSkillId),
            "TriggerBullet" => Some(ConditionType::TriggerBullet),
            "TeammateInjuryCountNotReset" => Some(ConditionType::TeammateInjuryCountNotReset {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1),
            }),
            "BeAttacked" => Some(ConditionType::BeAttacked),
            "BloodPoolValue" | "BloodPoolCompare" => Some(ConditionType::BloodPool),
            "HurtNotRestraint" => Some(ConditionType::HurtNotRestraint),
            "CanUseSkill" => Some(ConditionType::CanUseSkill),
            "TeamInjuryCountRound" => Some(ConditionType::TeamInjuryCountRound),
            "TeammateInjuryCount" => Some(ConditionType::TeammateInjuryCount {
                threshold: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(1),
            }),
            "PowerCompare" => Some(ConditionType::PowerCompare),
            "HeroRoundInterval" => Some(ConditionType::HeroRoundInterval {
                // `45104#start_round#period` — empirical: boss/passive rows like
                // `45104#1#1`, `2#2`, `3#3` fire only on the matching round in
                // LIVE replays. Evaluator treats matched-pair (start==period)
                // as "fire on round start_round only".
                start_round: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                period: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            "Dead" => Some(ConditionType::Dead),
            "ActiveUseSkill" => Some(ConditionType::ActiveUseSkill),
            "ActiveUseSkillId" => Some(ConditionType::ActiveUseSkillId {
                skill_ids: parts
                    .get(1)
                    .map(|p| p.split(',').filter_map(|v| v.parse().ok()).collect())
                    .unwrap_or_default(),
            }),
            "NoActRound" => Some(ConditionType::NoActRound),
            "TeammateUseExSkill" => Some(ConditionType::TeammateUseExSkill),
            "BattleTagNum" => Some(ConditionType::BattleTagNum {
                tag_id: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                threshold: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            }),
            _ => None,
        }
    }

    fn check(&self, condition: &ConditionType, _ctx: &ConditionEval<'_>) -> Option<bool> {
        // Event-driven conditions are now handled by PhaseFilter::check_combat via CombatEvent.
        // This function only handles conditions that are unconditionally false in combat.
        match condition {
            ConditionType::UseExSkill
            | ConditionType::UseSkillId
            | ConditionType::TriggerBullet
            | ConditionType::TeammateInjuryCountNotReset { .. }
            | ConditionType::BloodPool
            | ConditionType::CanUseSkill
            | ConditionType::PowerCompare
            | ConditionType::Dead
            | ConditionType::NoActRound
            | ConditionType::TeammateUseExSkill => Some(false),
            _ => None,
        }
    }
}
