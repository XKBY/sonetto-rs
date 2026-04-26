use super::ConditionEval;
use super::ConditionType;
use super::action::Condition;

pub(super) struct EnterFight;

impl Condition for EnterFight {
    fn parse(parts: &[&str], cond_type: &str) -> Option<ConditionType> {
        // The first part is the raw condition id; the rest live in
        // sibling clusters. EnterFight only needs the id, so it
        // extracts it here from `parts[0]`.
        let id: i32 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);

        // ID 6 is tagged `type=None` in config but is semantically the
        // "Unconditional battle-start" gate (not a combat always-pass). Treat
        // it as EnterFight here so Combat-phase passes correctly reject it;
        // use 210 for the true combat-None gate.
        if matches!(id, 5 | 5021 | 6) {
            return Some(ConditionType::EnterFight { condition_id: id });
        }
        match cond_type {
            "EnterFight" => Some(ConditionType::EnterFight { condition_id: id }),
            "None" => match id {
                // These None conditions are combat triggers, not battle-start passives.
                210 => Some(ConditionType::CombatNone),
                _ => Some(ConditionType::None),
            },
            _ => None,
        }
    }

    fn check(condition: &ConditionType, ctx: &ConditionEval<'_>) -> Option<bool> {
        match condition {
            ConditionType::None | ConditionType::CombatNone | ConditionType::EnterFight { .. } => {
                Some(true)
            }
            ConditionType::EnterFightAnd(_) | ConditionType::EnterFightOr(_) => {
                Some(super::fold(condition, &mut |cond| {
                    super::check_condition(
                        ctx.fight,
                        ctx.buff_mgr,
                        ctx.ex_point_mgr,
                        ctx.bloodtithe,
                        ctx.caster_uid,
                        ctx.target_uid,
                        ctx.has_trigger_state,
                        cond,
                    )
                }))
            }
            _ => None,
        }
    }
}
