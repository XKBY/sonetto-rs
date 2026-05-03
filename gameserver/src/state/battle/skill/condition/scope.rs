//! Condition-ID firing scope (the "where does this rule run?" axis
//! that the engine encodes via the condition row's id, not its type
//! field).
//!
//! ## Why a side-table?
//!
//! `skill_behavior_condition.json` has many rows whose `type` field
//! is `"None"` but whose IDs almost certainly mean different firing
//! contexts inside the engine:
//!
//! * `c100` — "at the start of the round" (`30090146` Sotheby
//!   Duality Potion, `31040141` Willow start-of-round Poison)
//! * `c101` — same round-start scope (`30980151` Tuesday's Insight
//!   III "resolve 1 round of Poison")
//! * `c104` — same round-start scope (`30630171` Pickles "gain 1
//!   stack of Clarified Topic")
//! * `c208` — inline action / boss-reactive emission default
//! * `c210` — inline action default for player skills (see
//!   `30090111`/`31020121` etc.)
//! * `c203`/`c201` — same-side action reactive
//!
//! Our parser collapses all "None"-typed ids to `ConditionType::None`,
//! losing the discriminator. This module recovers it via the cached
//! `ResolvedBehavior::condition_id` (which the parser already
//! preserves).
//!
//! ## Usage
//!
//! Anywhere we'd want to gate "fires at round-start only" behavior
//! (today: would have unblocked `60073 SettleDotAndCostDotDuration`'s
//! double-fire bug without inventing a behavior-side `has_round_sweep_
//! behavior` predicate), call `condition_scope(b.condition_id)` and
//! match the resulting `ConditionScope`.
//!
//! ## Empirical basis
//!
//! IDs were classified by `scripts/condition_scope_discovery.py` —
//! group skills sharing each condition id, read their in-game
//! descriptions, and look at LIVE's emission depth/parent context.
//! Round-start IDs all said "At the start of the round" and emitted
//! at `depth=2 parent_aid=0` (top-level round wrappers). Inline IDs
//! emit at varied depths under skill parents and never have round-
//! start phrasing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionScope {
    /// Fires whenever the skill is evaluated. Default for unmapped
    /// ids — preserves current "always-fire" behavior so adding new
    /// id classifications doesn't regress existing wired primitives.
    Always,
    /// Fires only during the round manager's start-of-round passive
    /// sweep. Must be blocked from inline card execution and
    /// combat-trigger replay.
    RoundStart,
}

/// Map a `skill_behavior_condition.id` to its firing scope. IDs not
/// in the table fall through to `Always`, preserving back-compat with
/// the current "None means unconditional" default.
pub fn condition_scope(condition_id: i32) -> ConditionScope {
    match condition_id {
        // Round-start sweep — descriptions all read "At the start of
        // the round, [...]" and LIVE emits these at depth=2 under a
        // parent_aid=0 round wrapper.
        100 | 101 | 104 => ConditionScope::RoundStart,
        _ => ConditionScope::Always,
    }
}

/// Convenience predicate for trigger pipeline gates that need to
/// skip round-start-only passives during inline card execution and
/// combat-trigger replay.
pub fn is_round_start_only(condition_id: i32) -> bool {
    matches!(condition_scope(condition_id), ConditionScope::RoundStart)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_start_ids_classified() {
        assert_eq!(condition_scope(100), ConditionScope::RoundStart);
        assert_eq!(condition_scope(101), ConditionScope::RoundStart);
        assert_eq!(condition_scope(104), ConditionScope::RoundStart);
        assert!(is_round_start_only(100));
        assert!(is_round_start_only(101));
        assert!(is_round_start_only(104));
    }

    #[test]
    fn unmapped_ids_default_to_always() {
        // Inline-action ids — should NOT be RoundStart.
        for id in [0, 5, 100, 203, 208, 210, 591].iter().copied() {
            if id == 100 {
                continue;
            }
            assert_ne!(
                condition_scope(id),
                ConditionScope::RoundStart,
                "id {id} should not be RoundStart"
            );
        }
        assert_eq!(condition_scope(208), ConditionScope::Always);
        assert_eq!(condition_scope(210), ConditionScope::Always);
        assert_eq!(condition_scope(203), ConditionScope::Always);
        assert!(!is_round_start_only(208));
        assert!(!is_round_start_only(0));
    }
}
