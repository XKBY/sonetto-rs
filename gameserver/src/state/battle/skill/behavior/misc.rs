use anyhow::Result;
use sonettobuf::ActEffect;

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;

/// Misc action — handler for the behavior variants that are
/// currently no-ops or simple skill-execution placeholders. Each
/// variant either emits nothing or logs a warning.
///
/// Variants owned:
/// * `Summon { .. }` — queues a silent defender-side entity spawn.
/// * `MonsterChange { .. }` — applies entity form swap via
///   `mechanics::phase_change::transform_entity`.
/// * `Kill`, `ShellUseSkill { .. }`, `ShellAssign { .. }`,
///   `BeAttackedAssassinate { .. }`, `CrystalAddCard` — placeholders.
/// * `IgnoreSkillConfigDamageRate` — flag-only behavior; suppression
///   happens elsewhere in the executor.
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
            BehaviorType::Summon {
                skill_id: monster_id,
            } => {
                ctx.executor.pending_summons.push(
                    crate::state::battle::skill::executor::PendingSummon {
                        caster_uid: ctx.caster_uid,
                        monster_id: *monster_id,
                    },
                );
                Some(Ok(vec![]))
            }
            BehaviorType::MonsterChange {
                new_monster_id,
                probability_permille,
            } => {
                // Probability gate: 1000 = always, sub-1000 needs RNG.
                // Treat anything ≥1000 as deterministic; deterministic
                // sub-1000 cases haven't surfaced in our fixtures yet.
                if *probability_permille < 1000 {
                    tracing::trace!(
                        "MonsterChange probability_permille={} treated as deterministic",
                        probability_permille
                    );
                }
                ctx.executor.pending_monster_changes.push(
                    crate::state::battle::skill::executor::PendingMonsterChange {
                        target_uid: ctx.target,
                        new_monster_id: *new_monster_id,
                    },
                );
                Some(Ok(vec![]))
            }
            BehaviorType::Kill
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
