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
//! * `c100` / `c101` / `c102` / `c104` — "at the start of the round"
//!   (`30090146` Sotheby Duality Potion, `30980151` Tuesday's Insight
//!   III "resolve 1 round of Poison", `30630171` Pickles "gain 1
//!   stack of Clarified Topic", and a long tail of round-start
//!   passive auras — see "Empirical basis" below).
//! * `c208` — inline action / boss-reactive emission default
//! * `c210` — inline action default for player skills (see
//!   `30090111`/`31020121` etc.)
//! * `c203`/`c201` — same-side action reactive
//! * `c301` / `c302` / `c303` / `c304` / `c307` — "at the end of the
//!   round" (`2107`, `2114`, `2363`, `2415`, `2506`, …). Currently
//!   classified as `RoundEnd` purely on description evidence — none
//!   of the fixture battles exercise these so emit-context evidence
//!   is pending.
//!
//! Our parser collapses all "None"-typed ids to `ConditionType::None`,
//! losing the discriminator. This module recovers it via the cached
//! `ResolvedBehavior::condition_id` (which the parser already
//! preserves).
//!
//! ## Usage
//!
//! Anywhere we'd want to gate "fires at round-start only" behavior
//! (today: `60073 SettleDotAndCostDotDuration`'s double-fire bug, the
//! generic `30090146`/`30980151`/`30630171` over-fire from inline card
//! paths), call `condition_scope(b.condition_id)` and match the
//! resulting `ConditionScope`.
//!
//! Round-end scoping is documented but not yet wired into a gate —
//! the engine doesn't currently have a round-end-only sweep that
//! needs to consult this. When that lands, use `is_round_end_only`
//! analogous to `is_round_start_only`.
//!
//! ## Empirical basis
//!
//! IDs were classified by `scripts/condition_scope_discovery.py` —
//! group skills sharing each condition id, read their in-game
//! descriptions, and look at LIVE's emission depth/parent context.
//!
//! `c100/c101/c104` round-start IDs all said "At the start of the
//! round" and emitted at `depth=2 parent_aid=0` (top-level round
//! wrappers). Inline IDs emit at varied depths under skill parents
//! and never have round-start phrasing.
//!
//! `c102` was added on description evidence alone (217/272 ≈ 80% of
//! skills using `c102` carry round-start phrasing in their visible
//! description; the remaining "other" cases are passive auras that
//! re-fire each round-start under different wording like "Starts the
//! round with X" or "Starts a round in [Break Time] status"). No
//! fixture battle exercises a `c102` skill today, so the parity
//! impact of this classification is zero. Promoted to `RoundStart`
//! anyway because the engine evidently reuses round-start scope IDs
//! 100/101/102/103/104 as a contiguous block.
//!
//! `c103` was considered but kept at `Always` because ~30% of its
//! skills are passive-aura statlines like "Enhance [Break Time]
//! effect: DMG Taken Reduction +15%" which would mis-fire if forced
//! to round-start.
//!
//! `c301`–`c307` were classified `RoundEnd` based on description
//! evidence: 81–97% of skills using each id carry "at the end of the
//! round" / "when a round ends" phrasing. `c307` specifically is the
//! "every N rounds" variant ("At the end of every 3 rounds, …"). No
//! fixture battle exercises these today, so this is defensive
//! infrastructure for the eventual round-end gate.

use crate::state::battle::skill::cache::resolve_skill_effect_id;

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
    /// Fires only during the round manager's end-of-round passive
    /// sweep. Currently no engine gate consults this (no round-end
    /// fixture skills fire today), but the classification is
    /// documented so the gate can read it cleanly when wired.
    RoundEnd,
}

/// Map a `skill_behavior_condition.id` to its firing scope. IDs not
/// in the table fall through to `Always`, preserving back-compat with
/// the current "None means unconditional" default.
pub fn condition_scope(condition_id: i32) -> ConditionScope {
    match condition_id {
        // Round-start sweep — descriptions all read "At the start of
        // the round, [...]" and LIVE emits these at depth=2 under a
        // parent_aid=0 round wrapper.
        100 | 101 | 102 | 104 => ConditionScope::RoundStart,
        // Round-end sweep — descriptions all read "When a round
        // ends, [...]" / "At the end of the round, [...]" / "At the
        // end of every N rounds, [...]" (c307). Description-only
        // classification; no fixture coverage yet.
        301 | 302 | 303 | 304 | 307 => ConditionScope::RoundEnd,
        _ => ConditionScope::Always,
    }
}

/// Convenience predicate for trigger pipeline gates that need to
/// skip round-start-only passives during inline card execution and
/// combat-trigger replay.
pub fn is_round_start_only(condition_id: i32) -> bool {
    matches!(condition_scope(condition_id), ConditionScope::RoundStart)
}

/// Convenience predicate for the (future) round-end gate. Today no
/// caller wires this — it exists for parity with `is_round_start_only`
/// so the eventual round-end pipeline doesn't have to re-derive the
/// classification.
pub fn is_round_end_only(condition_id: i32) -> bool {
    matches!(condition_scope(condition_id), ConditionScope::RoundEnd)
}

/// Extract the leading numeric id from a raw condition string like
/// `"100"` or `"203&201"`.
pub(crate) fn first_condition_id(raw: &str) -> Option<i32> {
    let s = raw.trim_start_matches('!').trim_start_matches('！');
    let head_end = s
        .find(|c: char| c == '#' || c == '&' || c == '|' || c == '!' || c == '！')
        .unwrap_or(s.len());
    s[..head_end].trim().parse::<i32>().ok()
}

/// True when the resolved skill effect is a single-slot pure `c100`
/// passive: `condition1` is exactly raw `100`, and all later
/// condition slots are empty.
pub(crate) fn is_single_slot_pure_c100_passive(skill_id: i32) -> bool {
    let effect_id = resolve_skill_effect_id(skill_id);
    let Some(skill) = config::configs::get().skill_effect.get(effect_id) else {
        return false;
    };

    if first_condition_id(&skill.condition1) != Some(100) || skill.condition1.trim() != "100" {
        return false;
    }

    [
        skill.condition2.as_str(),
        skill.condition3.as_str(),
        skill.condition4.as_str(),
        skill.condition5.as_str(),
        skill.condition6.as_str(),
        skill.condition7.as_str(),
        skill.condition8.as_str(),
        skill.condition9.as_str(),
        skill.condition10.as_str(),
        skill.condition11.as_str(),
        skill.condition12.as_str(),
        skill.condition13.as_str(),
        skill.condition14.as_str(),
        skill.condition15.as_str(),
        skill.condition16.as_str(),
        skill.condition17.as_str(),
        skill.condition18.as_str(),
        skill.condition19.as_str(),
        skill.condition20.as_str(),
    ]
    .into_iter()
    .all(|raw| raw.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_start_ids_classified() {
        for id in [100, 101, 102, 104] {
            assert_eq!(
                condition_scope(id),
                ConditionScope::RoundStart,
                "id {id} should be RoundStart"
            );
            assert!(is_round_start_only(id));
            assert!(!is_round_end_only(id));
        }
    }

    #[test]
    fn round_end_ids_classified() {
        for id in [301, 302, 303, 304, 307] {
            assert_eq!(
                condition_scope(id),
                ConditionScope::RoundEnd,
                "id {id} should be RoundEnd"
            );
            assert!(is_round_end_only(id));
            assert!(!is_round_start_only(id));
        }
    }

    #[test]
    fn unmapped_ids_default_to_always() {
        // Inline-action / event-driven ids — should NOT be RoundStart
        // or RoundEnd. These are the "None"-typed catch-alls the
        // engine uses for skill-execution and combat-event hooks.
        for id in [0, 5, 55, 103, 106, 201, 203, 208, 210, 591]
            .iter()
            .copied()
        {
            assert_eq!(
                condition_scope(id),
                ConditionScope::Always,
                "id {id} should be Always"
            );
            assert!(!is_round_start_only(id));
            assert!(!is_round_end_only(id));
        }
    }
}
