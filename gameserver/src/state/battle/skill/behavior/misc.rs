use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Misc action — handler for the behavior variants that are
/// currently no-ops or simple skill-execution placeholders. Each
/// variant either emits nothing or logs a warning.
///
/// Variants owned (all currently emit `Ok(vec![])`):
/// * `Summon { .. }` — queues a silent defender-side entity spawn.
/// * `Kill` — placeholder.
/// * `MonsterChange` — placeholder.
/// * `ShellUseSkill { .. }` — Shell-system placeholder.
/// * `ShellAssign { .. }` — Shell-system placeholder.
/// * `BeAttackedAssassinate { .. }` — placeholder.
/// * `CrystalAddCard` — placeholder.
/// * `IgnoreSkillConfigDamageRate` — flag-only behavior; the actual
///   suppression happens elsewhere in the executor.
/// * `Unknown { raw }` — log and skip.
pub(super) struct Misc;

impl BehaviorAction for Misc {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>> {
        match behavior {
            BehaviorType::Summon { skill_id: monster_id } => {
                ctx.executor
                    .pending_summons
                    .push(crate::state::battle::skill::executor::PendingSummon {
                        caster_uid: ctx.caster_uid,
                        monster_id: *monster_id,
                    });
                Some(Ok(vec![]))
            }
            BehaviorType::Kill
            | BehaviorType::MonsterChange
            | BehaviorType::ShellUseSkill { .. }
            | BehaviorType::ShellAssign { .. }
            | BehaviorType::BeAttackedAssassinate { .. }
            | BehaviorType::CrystalAddCard
            | BehaviorType::IgnoreSkillConfigDamageRate => Some(Ok(vec![])),
            BehaviorType::Unknown { raw } => {
                tracing::warn!("Skipping unknown behavior: {}", raw);
                Some(Ok(vec![]))
            }
            _ => None,
        }
    }
}
