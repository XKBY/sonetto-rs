//! Read-only emission accounting for runtime debugging.
//!
//! Records every skill emission with provenance metadata
//! (which code path emitted it, who owns it, what triggered it).
//! Records are appended in chronological order — the structure is
//! literally a timeline of "what fired and from where" for one round.
//!
//! The timeline never affects engine behavior. It is round-scoped,
//! cleared at round-open, populated as emissions happen, and dumped
//! to stderr at round-end when `SONETTO_EMISSION_TIMELINE=1` is set
//! in the environment. Without that env var, recording is a no-op
//! beyond the `Vec::push`.
//!
//! ## Why this exists
//!
//! The engine has 5+ paths that can emit a skill (card-cast inline
//! self-passive walk, behavior dispatch, combat trigger expansion,
//! per-uid passive sweep, dedicated battle-rule pass, magic-circle
//! enemy-skill walk). When LIVE captures show a skill firing once
//! and OURS shows it firing four times, finding which paths
//! over-fired is currently a manual `eprintln!` archaeology run.
//! This module turns that archaeology into structured evidence the
//! engine emits on demand.
//!
//! ## Read order
//!
//! - `EmissionPhase` — the enum identifying which code path emitted.
//! - `EmissionRecord` — a single (phase, owner, skill, trigger) tuple.
//! - `EmissionTimeline` — the chronological list of records, with
//!   convenience accessors for spotting duplicates.

use std::collections::HashMap;

/// Which code path emitted a given skill. Each variant maps to a
/// concrete call site in the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmissionPhase {
    /// `card_mgr::play_card` resolves a player operation into a
    /// SKILL step. The host card cast itself.
    CardCast,
    /// `card_mgr` inline active-use passive loop after the host
    /// card resolves but before the host step is materialized.
    CardInlinePassive,
    /// `trigger::combat::run_combat_passives_pass` — fires per-uid
    /// reactive passives in response to a player or enemy event.
    TriggerCombatPassive,
    /// `round_mgr::run_passive_phase` with `ExcludeBattleRule` —
    /// the per-uid round-level passive sweep.
    RoundPassiveSweep,
    /// `round_mgr::run_passive_phase` with `BattleRuleOnly` — the
    /// dedicated attacker-side battle-rule pass.
    BattleRuleOnly,
    /// `round_mgr::run_passive_phase` with `DefenderBootstrap` —
    /// the defender-side bootstrap pass at round open.
    DefenderBootstrap,
    /// `round_mgr::run_passive_phase` with `CombatReactive` — the
    /// post-action combat-reactive sweep.
    CombatReactive,
    /// `passives::executor::run_battle_start` — initial battle-start
    /// passive emission (not used in replay path).
    BattleStart,
    /// `mechanics::channel` — Sentinel monitor-continue chain.
    ChannelMechanic,
    /// `magic_circle` enemy-skills walker.
    MagicCircleEnemy,
    /// Buff-feature reactive emission (BloodValueUseSkill, etc.).
    BuffFeatureReactive,
    /// Psychube emission as a depth-1 inline child of a host card
    /// cast. Recognized via `equipment::is_psychube_skill` —
    /// any record-site that would otherwise tag a path-based phase
    /// (TriggerCombatPassive, RoundPassiveSweep, etc.) reclassifies
    /// to this variant when the skill_id matches an `equip_skill`
    /// row. Lets the duplicates table separate "psychube via
    /// inline attachment" (LIVE-correct shape) from "psychube via
    /// sweep duplication" (bandaid duplicate).
    PsychubeAttached,
    /// Catch-all: every call into `skill::executor::execute_skill`
    /// is recorded here. This is the lowest-level emission funnel
    /// — every skill emission ultimately passes through it. Higher
    /// level phases (CardCast, TriggerCombatPassive, etc.) will
    /// also record their own entry, so most skills appear with both
    /// a high-level and a low-level record. Skills that ONLY appear
    /// with `ExecutorLowLevel` are emissions whose call site doesn't
    /// have a dedicated phase tag yet — those are the "missing
    /// coverage" cases that prior timeline-driven fix attempts
    /// couldn't suppress.
    ExecutorLowLevel,
    /// `skill/behavior/direct_skill::execute_direct_use_big_skill`
    /// passive-fanout loop. Each skill the DUGSS body fans out (the
    /// passive_skill list of the active-use caster, the EX-skill
    /// body's chained reactives) is recorded here.
    DirectUseBigSkillFanout,
    /// `passives/inject.rs` recursive walker that grafts ally
    /// reactives into emitted enemy fightStep subtrees. Operates on
    /// the already-built tree.
    AllyReactiveInject,
    /// Post-emission graft from
    /// `round_mgr::graft_be_attacked_reactives_onto_player_host` —
    /// synthesizes a wrapper directly without going through
    /// `execute_skill`. Recorded separately so the timeline still
    /// sees it.
    BeAttackedGraft,
}

impl EmissionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            EmissionPhase::CardCast => "CardCast",
            EmissionPhase::CardInlinePassive => "CardInlinePassive",
            EmissionPhase::TriggerCombatPassive => "TriggerCombatPassive",
            EmissionPhase::RoundPassiveSweep => "RoundPassiveSweep",
            EmissionPhase::BattleRuleOnly => "BattleRuleOnly",
            EmissionPhase::DefenderBootstrap => "DefenderBootstrap",
            EmissionPhase::CombatReactive => "CombatReactive",
            EmissionPhase::BattleStart => "BattleStart",
            EmissionPhase::ChannelMechanic => "ChannelMechanic",
            EmissionPhase::MagicCircleEnemy => "MagicCircleEnemy",
            EmissionPhase::BuffFeatureReactive => "BuffFeatureReactive",
            EmissionPhase::PsychubeAttached => "PsychubeAttached",
            EmissionPhase::ExecutorLowLevel => "ExecutorLowLevel",
            EmissionPhase::DirectUseBigSkillFanout => "DirectUseBigSkillFanout",
            EmissionPhase::AllyReactiveInject => "AllyReactiveInject",
            EmissionPhase::BeAttackedGraft => "BeAttackedGraft",
        }
    }
}

/// One emission, recorded chronologically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmissionRecord {
    /// Which code path emitted this skill.
    pub phase: EmissionPhase,
    /// Owner of the emitted skill (the entity in whose passive list
    /// the skill lives, OR the card caster for `CardCast`).
    pub owner_uid: i64,
    /// Skill id being emitted.
    pub skill_id: i32,
    /// Action order index of the triggering event, if any.
    /// 0 means "no specific action" (round-open / round-end / idle
    /// sweep).
    pub action_order_index: i32,
    /// Skill id of the host event that triggered this emission, if
    /// any. None for `CardCast` (the cast IS the trigger) and for
    /// idle-sweep emissions.
    pub triggered_by_skill_id: Option<i32>,
    /// Caster of the host event that triggered this emission, if
    /// any. None for `CardCast` and idle-sweep emissions.
    pub triggered_by_caster_uid: Option<i64>,
    /// Whether this emission produced non-empty output (i.e. the
    /// underlying `execute_skill` call returned at least one
    /// `ActEffect`, OR the call site directly constructed a wrapper).
    /// `false` initially; set to `true` via `mark_produced` after
    /// the emission completes if output is non-empty.
    ///
    /// **Why this matters**: a record with `produced_output=false`
    /// is "intent without effect" — the engine considered emitting
    /// but produced nothing (e.g. condition gates rejected the
    /// emission inside `execute_skill`). Suppressing such a record
    /// has no audit effect because the path wasn't producing output.
    /// The duplication report distinguishes these from real
    /// emissions so future fix attempts target the right paths.
    pub produced_output: bool,
}

/// Chronological record of every skill emission within one round.
///
/// Cleared at round-open; populated as emissions happen; dumped at
/// round-end when `SONETTO_EMISSION_TIMELINE=1` is set in the
/// environment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmissionTimeline {
    pub round_index: i32,
    pub entries: Vec<EmissionRecord>,
}

impl EmissionTimeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset for a new round. Call at round-open.
    pub fn reset(&mut self, round_index: i32) {
        self.round_index = round_index;
        self.entries.clear();
    }

    /// Append one emission. Returns the index of the new entry —
    /// pass it to `mark_produced` after the emission completes if
    /// output was non-empty.
    ///
    /// Psychube emissions (skills in the equipment range that have
    /// a corresponding `skill_effect`/`skill_buff` config row) are
    /// automatically reclassified from path-based phases to
    /// `PsychubeAttached`, regardless of which call site recorded
    /// them. This keeps the call sites simple and ensures the
    /// duplicates table separates psychube emissions from generic
    /// hero-passive emissions even when both fire from the same
    /// passive-sweep path.
    pub fn record(
        &mut self,
        phase: EmissionPhase,
        owner_uid: i64,
        skill_id: i32,
        action_order_index: i32,
        triggered_by_skill_id: Option<i32>,
        triggered_by_caster_uid: Option<i64>,
    ) -> usize {
        let phase = if phase != EmissionPhase::CardCast
            && crate::state::battle::equipment::is_psychube_skill(skill_id)
        {
            EmissionPhase::PsychubeAttached
        } else {
            phase
        };
        let idx = self.entries.len();
        self.entries.push(EmissionRecord {
            phase,
            owner_uid,
            skill_id,
            action_order_index,
            triggered_by_skill_id,
            triggered_by_caster_uid,
            produced_output: false,
        });
        idx
    }

    /// Mark a previously-recorded entry as having produced
    /// non-empty output. Call after `execute_skill` (or equivalent)
    /// returns and the result is known to be non-empty. No-op if
    /// the index is out of bounds.
    pub fn mark_produced(&mut self, idx: usize) {
        if let Some(rec) = self.entries.get_mut(idx) {
            rec.produced_output = true;
        }
    }

    /// Group entries by `(skill_id, owner_uid)` and return groups
    /// where the same `(skill_id, owner_uid)` was emitted more than
    /// once — these are the duplication candidates.
    pub fn duplicates_by_owner_skill(&self) -> Vec<DuplicateGroup> {
        let mut groups: HashMap<(i32, i64), Vec<EmissionPhase>> = HashMap::new();
        for record in &self.entries {
            groups
                .entry((record.skill_id, record.owner_uid))
                .or_default()
                .push(record.phase);
        }
        let mut out: Vec<DuplicateGroup> = groups
            .into_iter()
            .filter(|(_, phases)| phases.len() > 1)
            .map(|((skill_id, owner_uid), phases)| DuplicateGroup {
                skill_id,
                owner_uid,
                phases,
            })
            .collect();
        out.sort_by(|a, b| a.skill_id.cmp(&b.skill_id).then(a.owner_uid.cmp(&b.owner_uid)));
        out
    }

    /// Group entries by `skill_id` ignoring owner — useful for
    /// spotting "one skill resolved through multiple paths" even
    /// when emitted for different owners (e.g. boss state cycle
    /// `530000151` running for every enemy in the same round).
    ///
    /// Returns separate counts for total intent and produced
    /// output. `produced_count > 0` means at least one record for
    /// this skill produced FightStep output; `produced_count <
    /// count` means some recorded calls were intent-without-output
    /// (suppressing those would have no audit effect).
    pub fn duplicates_by_skill(&self) -> Vec<SkillDuplicateGroup> {
        let mut groups: HashMap<i32, Vec<&EmissionRecord>> = HashMap::new();
        for record in &self.entries {
            groups.entry(record.skill_id).or_default().push(record);
        }
        let mut out: Vec<SkillDuplicateGroup> = groups
            .into_iter()
            .filter(|(_, recs)| recs.len() > 1)
            .map(|(skill_id, recs)| {
                let produced_count = recs.iter().filter(|r| r.produced_output).count();
                let mut produced_phases: Vec<EmissionPhase> = recs
                    .iter()
                    .filter(|r| r.produced_output)
                    .map(|r| r.phase)
                    .collect();
                produced_phases.sort_by_key(|p| p.as_str());
                produced_phases.dedup();
                let mut all_phases: Vec<EmissionPhase> =
                    recs.iter().map(|r| r.phase).collect();
                all_phases.sort_by_key(|p| p.as_str());
                all_phases.dedup();
                SkillDuplicateGroup {
                    skill_id,
                    count: recs.len(),
                    produced_count,
                    phases: all_phases,
                    produced_phases,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.produced_count
                .cmp(&a.produced_count)
                .then(b.count.cmp(&a.count))
                .then(a.skill_id.cmp(&b.skill_id))
        });
        out
    }

    /// Render the timeline as a human-readable dump. Called at
    /// round-end when `SONETTO_EMISSION_TIMELINE=1`.
    pub fn dump(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let produced_total = self.entries.iter().filter(|r| r.produced_output).count();
        let _ = writeln!(
            out,
            "=== EmissionTimeline round={} entries={} produced={} ===",
            self.round_index,
            self.entries.len(),
            produced_total,
        );
        for (i, record) in self.entries.iter().enumerate() {
            let trigger = match (record.triggered_by_skill_id, record.triggered_by_caster_uid) {
                (Some(sid), Some(uid)) => format!(" via skill={} caster={}", sid, uid),
                _ => String::new(),
            };
            let out_marker = if record.produced_output { "[OUT]" } else { "[   ]" };
            let _ = writeln!(
                out,
                "  [{:3}] {} phase={:<24} owner={:>11} skill={:>9} order={}{}",
                i,
                out_marker,
                record.phase.as_str(),
                record.owner_uid,
                record.skill_id,
                record.action_order_index,
                trigger
            );
        }
        let dup_skill = self.duplicates_by_skill();
        if !dup_skill.is_empty() {
            let _ = writeln!(
                out,
                "--- duplicates by skill_id ({} skills) — count(intent) / produced ---",
                dup_skill.len()
            );
            for group in &dup_skill {
                let phases: Vec<&str> = group.phases.iter().map(|p| p.as_str()).collect();
                let produced_phases: Vec<&str> =
                    group.produced_phases.iter().map(|p| p.as_str()).collect();
                let _ = writeln!(
                    out,
                    "  skill={:>9} count={} produced={} phases=[{}] producing=[{}]",
                    group.skill_id,
                    group.count,
                    group.produced_count,
                    phases.join(", "),
                    produced_phases.join(", ")
                );
            }
        }
        out
    }

    /// Whether the env var `SONETTO_EMISSION_TIMELINE` is set to
    /// any non-empty value. Used by callers that print the dump.
    pub fn dump_enabled() -> bool {
        std::env::var("SONETTO_EMISSION_TIMELINE")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub skill_id: i32,
    pub owner_uid: i64,
    pub phases: Vec<EmissionPhase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDuplicateGroup {
    pub skill_id: i32,
    /// Total recorded intent (every `record()` call).
    pub count: usize,
    /// Subset of `count` whose `produced_output` flag is true.
    pub produced_count: usize,
    /// All phases that recorded this skill (intent-level).
    pub phases: Vec<EmissionPhase>,
    /// Subset of `phases` that produced non-empty output.
    pub produced_phases: Vec<EmissionPhase>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_records_in_order() {
        let mut t = EmissionTimeline::new();
        t.reset(1);
        t.record(EmissionPhase::CardCast, 100, 31140151, 1, None, None);
        t.record(
            EmissionPhase::TriggerCombatPassive,
            -1,
            530000151,
            1,
            Some(31140131),
            Some(100),
        );
        assert_eq!(t.entries.len(), 2);
        assert_eq!(t.entries[0].phase, EmissionPhase::CardCast);
        assert_eq!(t.entries[1].skill_id, 530000151);
    }

    #[test]
    fn duplicates_by_skill_groups_correctly() {
        let mut t = EmissionTimeline::new();
        t.record(
            EmissionPhase::TriggerCombatPassive,
            -1,
            530000151,
            1,
            Some(31140131),
            Some(100),
        );
        t.record(
            EmissionPhase::TriggerCombatPassive,
            -2,
            530000151,
            1,
            Some(31140131),
            Some(100),
        );
        t.record(
            EmissionPhase::DefenderBootstrap,
            -3,
            530000151,
            0,
            None,
            None,
        );
        t.record(EmissionPhase::CardCast, 100, 31140151, 1, None, None);

        let dups = t.duplicates_by_skill();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].skill_id, 530000151);
        assert_eq!(dups[0].count, 3);
    }

    #[test]
    fn reset_clears_entries() {
        let mut t = EmissionTimeline::new();
        t.record(EmissionPhase::CardCast, 100, 31140151, 1, None, None);
        assert_eq!(t.entries.len(), 1);
        t.reset(2);
        assert_eq!(t.entries.len(), 0);
        assert_eq!(t.round_index, 2);
    }
}
