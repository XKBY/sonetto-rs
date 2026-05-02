//! Nautika channel-cast post-emission cleanup.
//!
//! When Nautika is channeling, her channel-host buff (a
//! `NuoDiKaCastChannel`-tagged buff such as `31200193`) wraps a
//! round-end bundle that LIVE-side absorbs the ally team's
//! battle-rule state-cycle broadcast plus the matching enemy-side
//! deletion. Our engine emits the rebroadcasts at the top level by
//! default; this module folds them into the channel-host wrapper
//! and strips the now-redundant orphans so the outgoing FightStep
//! stream matches the official client shape.
//!
//! Three entry points called from `manager::round_mgr` in this order:
//! - `strip_duplicate_change_round_markers`: removes standalone
//!   `EffectType::CardDeckNum` duplicates that some round-end
//!   bundles emit alongside the leading marker.
//! - `consolidate_into_bundle`: walks post-round-end FightSteps,
//!   finds the channel-host bundle, migrates Semmelweis's
//!   `530000151` wrapper into it, and strips the orphan ally
//!   broadcasts plus enemy-side `530000412` deletions whose data is
//!   now redundant.
//! - `strip_redundant_post_round_emissions`: removes top-level
//!   enemy boss-cycle rebroadcasts and the flat post-round
//!   state-marker wrappers that LIVE folds into earlier output.
//!
//! The actual Embrace the Past psychube (`equip_id = 1548`) ships
//! different rules — entry-time Max HP, damage-taken Crit Rate
//! stacks, conditional Crit DMG below 80% HP — and is unimplemented
//! today. None of this file relates to those amplification effects.

use std::collections::{HashMap, HashSet, VecDeque};

use config::{configs, skill_effect::SkillEffect};
use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    fight_step::ActEffectBuilder, heroes::nautika::CHANNEL_HOST_SKILL_IDS,
    skill::cache::resolve_skill_effect_id, step_walker, types::effects::EffectType,
    utils::find_uid_by_hero_id,
};

/// The signature prefix the data tables use for battle-rule
/// "state-cycle" skills/buffs (`530000xxx`). The transitive walk in
/// `discover_battle_rule_cycle_act_ids` only follows AddBuff edges
/// whose target id falls inside this range — keeps the closure
/// from leaking out of the rule chain into unrelated buffs that
/// the chain happens to mention.
const BATTLE_RULE_CYCLE_ID_MIN: i32 = 530000000;
const BATTLE_RULE_CYCLE_ID_MAX_EXCLUSIVE: i32 = 530001000;

/// Round-state marker effect types that LIVE emits alongside the
/// leading `CardDeckNum` opener but doesn't repeat at the top
/// level after the channel-host bundle is consolidated. Our engine
/// emits them naturally from the round-end pipeline; the cleanup
/// strips the redundant copies. The canonical names come from
/// the `EffectType` enum.
const POST_ROUND_STATE_MARKER_TYPES: [EffectType; 4] = [
    EffectType::DealCard2,
    EffectType::RoundEnd,
    EffectType::ClearUniversalCard,
    EffectType::SmallRoundEnd,
];

/// Hero ids (model_id) used to locate runtime uids of the broadcast
/// owners. The literals here are stable engine identifiers; see
/// memory note `project_battle1_gaps.md` re: `find_uid_by_hero_id`.
pub const SEMMELWEIS_HERO_ID: i32 = 3088;
pub const NAUTIKA_HERO_ID: i32 = 3120;

/// Walk post-round-end FightSteps, fold the active battle-rule
/// state-cycle broadcast into the Nautika channel-host bundle, and
/// strip the orphan ally rebroadcasts plus the enemy-side companion
/// deletions whose content is now redundant. The rule act ids come
/// from the active fight's `addition_rule` chain so the cleanup is
/// stage-aware.
pub fn consolidate_into_bundle(fight: &Fight, steps: &mut Vec<FightStep>) {
    #[derive(Clone, Copy)]
    struct WrapperLocation {
        step_idx: usize,
        effect_idx: usize,
        from_id: i64,
    }

    let rule_cycle = discover_battle_rule_cycle_act_ids(fight);
    if rule_cycle.root_act_ids.is_empty() {
        return;
    }

    let Some(semmelweis_uid) = find_uid_by_hero_id(fight, SEMMELWEIS_HERO_ID) else {
        return;
    };
    let Some(nautika_uid) = find_uid_by_hero_id(fight, NAUTIKA_HERO_ID) else {
        return;
    };

    if !steps.iter().any(|step| any_carrier_host_in_step(step)) {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(EffectType::AllocateCardEnergy as i32)
    }) else {
        return;
    };

    let Some(nautika_bundle_idx) = steps
        .iter()
        .position(|step| is_bundle_step(step, nautika_uid))
    else {
        return;
    };

    let mut ally_wrappers = Vec::new();
    let mut enemy_del_wrappers = Vec::new();

    for (step_idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if step_idx == nautika_bundle_idx
            || step.act_type != Some(fight_step::ActType::Effect as i32)
            || step.act_id.unwrap_or(0) != 0
        {
            continue;
        }

        for (effect_idx, effect) in step.act_effect.iter().enumerate() {
            let Some(skill) = step_walker::wrapped_skill_from_effect(effect) else {
                continue;
            };

            let act_id = skill.act_id.unwrap_or(0);
            let from_id = skill.from_id.unwrap_or(0);
            if rule_cycle.root_act_ids.contains(&act_id) && from_id > 0 {
                ally_wrappers.push(WrapperLocation {
                    step_idx,
                    effect_idx,
                    from_id,
                });
            } else if rule_cycle.enemy_companion_act_ids.contains(&act_id) && from_id < 0 {
                enemy_del_wrappers.push(WrapperLocation {
                    step_idx,
                    effect_idx,
                    from_id,
                });
            }
        }
    }

    let host_already_has_semm_broadcast = steps
        .get(nautika_bundle_idx)
        .map(|step| {
            step.act_effect.iter().any(|effect| {
                step_walker::wrapped_skill_from_effect(effect)
                    .map(|skill| {
                        skill
                            .act_id
                            .is_some_and(|act_id| rule_cycle.root_act_ids.contains(&act_id))
                            && skill.from_id == Some(semmelweis_uid)
                            && skill.to_id == Some(semmelweis_uid)
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let semm_wrapper = ally_wrappers
        .iter()
        .find(|wrapper| wrapper.from_id == semmelweis_uid)
        .copied();

    if !host_already_has_semm_broadcast
        && let Some(wrapper_loc) = semm_wrapper
        && let Some(source_step) = steps.get(wrapper_loc.step_idx)
        && let Some(source_effect) = source_step.act_effect.get(wrapper_loc.effect_idx)
        && let Some(mut normalized) = step_walker::normalize_wrapped_skill_effect(source_effect)
    {
        ensure_tail_marker(&mut normalized, semmelweis_uid, &rule_cycle.root_act_ids);
        if let Some(host_step) = steps.get_mut(nautika_bundle_idx) {
            host_step.act_effect.push(normalized);
        }
    }

    let mut removals_by_step: HashMap<usize, Vec<usize>> = HashMap::new();
    if host_already_has_semm_broadcast || semm_wrapper.is_some() {
        for wrapper in ally_wrappers {
            removals_by_step
                .entry(wrapper.step_idx)
                .or_default()
                .push(wrapper.effect_idx);
        }
    }
    for wrapper in enemy_del_wrappers {
        removals_by_step
            .entry(wrapper.step_idx)
            .or_default()
            .push(wrapper.effect_idx);
    }
    if removals_by_step.is_empty() {
        return;
    }

    let mut emptied_steps = Vec::new();
    for (step_idx, mut effect_indices) in removals_by_step {
        let Some(step) = steps.get_mut(step_idx) else {
            continue;
        };
        effect_indices.sort_unstable();
        effect_indices.dedup();
        for effect_idx in effect_indices.into_iter().rev() {
            if effect_idx < step.act_effect.len() {
                step.act_effect.remove(effect_idx);
            }
        }
        if step.act_effect.is_empty() {
            emptied_steps.push(step_idx);
        }
    }

    emptied_steps.sort_unstable();
    emptied_steps.dedup();
    for step_idx in emptied_steps.into_iter().rev() {
        steps.remove(step_idx);
    }
}

/// Remove duplicate `EffectType::CardDeckNum` (310) sync markers that
/// sometimes appear standalone after the round-leading marker when
/// the Nautika carrier-host is present. The first marker is kept as
/// the round opener; later standalone duplicates are stripped.
/// Returns silently when the carrier-host isn't on this round
/// (Nautika-only effect).
pub fn strip_duplicate_change_round_markers(steps: &mut Vec<FightStep>) {
    let change_round_sync = EffectType::CardDeckNum as i32;
    let Some(first_step) = steps.first() else {
        return;
    };
    if !step_walker::step_has_effect_type(first_step, change_round_sync) {
        return;
    }
    if !steps.iter().any(|step| any_carrier_host_in_step(step)) {
        return;
    }

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(1) {
        if step_walker::is_standalone_effect_marker(step, change_round_sync) {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// Remove top-level emissions LIVE folds into earlier output: the
/// enemy-side mirror of the active battle-rule cycle (rebroadcast
/// at the top level by our engine but absorbed by LIVE), and the
/// flat post-round state-marker wrappers (`DealCard2`, `RoundEnd`,
/// `ClearUniversalCard`, `SmallRoundEnd`) that LIVE pairs with the
/// leading `CardDeckNum` marker only. Runs after
/// `consolidate_into_bundle` has folded the canonical broadcast
/// into the carrier bundle.
pub fn strip_redundant_post_round_emissions(fight: &Fight, steps: &mut Vec<FightStep>) {
    let rule_cycle = discover_battle_rule_cycle_act_ids(fight);
    if rule_cycle.root_act_ids.is_empty() {
        return;
    }

    if !steps.iter().any(|step| any_carrier_host_in_step(step)) {
        return;
    }

    let Some(round_end_idx) = steps.iter().position(|step| {
        step.act_effect
            .first()
            .and_then(|effect| effect.effect_type)
            == Some(EffectType::AllocateCardEnergy as i32)
    }) else {
        return;
    };

    let mut remove_indices = Vec::new();
    for (idx, step) in steps.iter().enumerate().skip(round_end_idx + 1) {
        if is_redundant_enemy_cycle_rebroadcast(step, &rule_cycle.root_act_ids)
            || is_redundant_post_round_state_marker(step)
        {
            remove_indices.push(idx);
        }
    }

    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// True when `step` (or any descendant) carries an `act_id` that
/// belongs to Nautika's channel-cast host buff family.
fn any_carrier_host_in_step(step: &FightStep) -> bool {
    CHANNEL_HOST_SKILL_IDS
        .iter()
        .any(|host| step_walker::step_contains_act_id(step, *host))
}

fn is_bundle_step(step: &FightStep, host_uid: i64) -> bool {
    if step.act_type != Some(fight_step::ActType::Effect as i32) {
        return false;
    }

    let Some(first) = step.act_effect.first() else {
        return false;
    };
    if first.effect_type != Some(EffectType::FightStep as i32) {
        return false;
    }

    first
        .fight_step
        .as_ref()
        .map(|wrapped| {
            wrapped.act_type == Some(fight_step::ActType::Effect as i32)
                && wrapped
                    .act_id
                    .is_some_and(|id| CHANNEL_HOST_SKILL_IDS.contains(&id))
                && wrapped.from_id == Some(host_uid)
                && wrapped.to_id == Some(host_uid)
        })
        .unwrap_or(false)
}

fn ensure_tail_marker(wrapper: &mut ActEffect, semmelweis_uid: i64, root_act_ids: &HashSet<i32>) {
    let Some(skill) = step_walker::wrapped_skill_from_effect_mut(wrapper) else {
        return;
    };
    if !skill
        .act_id
        .is_some_and(|act_id| root_act_ids.contains(&act_id))
        || skill.from_id != Some(semmelweis_uid)
        || skill.to_id != Some(semmelweis_uid)
    {
        return;
    }
    let tail_marker = EffectType::Attr as i32;
    if skill
        .act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(tail_marker))
    {
        return;
    }
    if !skill
        .act_effect
        .iter()
        .any(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
    {
        return;
    }

    let insert_at = skill
        .act_effect
        .iter()
        .rposition(|effect| effect.effect_type == Some(EffectType::BuffUpdate as i32))
        .map(|idx| idx + 1)
        .unwrap_or(skill.act_effect.len());
    skill.act_effect.insert(
        insert_at,
        ActEffectBuilder::new(tail_marker, semmelweis_uid)
            .effect_num(0)
            .build(),
    );
}

fn is_redundant_enemy_cycle_rebroadcast(step: &FightStep, family_act_ids: &HashSet<i32>) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id.unwrap_or(0) == 0
        && step.from_id.unwrap_or(0) == 0
        && step.to_id.unwrap_or(0) == 0
        && !step.act_effect.is_empty()
        && step.act_effect.iter().all(|effect| {
            step_walker::wrapped_skill_from_effect(effect)
                .map(|skill| {
                    skill
                        .act_id
                        .is_some_and(|act_id| family_act_ids.contains(&act_id))
                        && skill.from_id.unwrap_or(0) < 0
                })
                .unwrap_or(false)
        })
}

fn is_redundant_post_round_state_marker(step: &FightStep) -> bool {
    step.act_type == Some(fight_step::ActType::Effect as i32)
        && step.act_id.unwrap_or(0) == 0
        && step.from_id.unwrap_or(0) == 0
        && step.to_id.unwrap_or(0) == 0
        && !step.act_effect.is_empty()
        && step.act_effect.len() <= 3
        && step.act_effect.iter().all(|effect| {
            effect.fight_step.is_none()
                && POST_ROUND_STATE_MARKER_TYPES
                    .iter()
                    .any(|marker| Some(*marker as i32) == effect.effect_type)
                && effect.target_id.unwrap_or(0) == 0
                && matches!(effect.effect_num.unwrap_or(0), 0 | 1)
        })
}

#[derive(Default)]
struct BattleRuleCycleActIds {
    /// Active rule effect skill ids — the explicit `rule[id].effect`
    /// for every entry in this fight's `battle.addition_rule`. These
    /// are what fire as ally-side state-cycle broadcasts (positive
    /// `from_id`).
    root_act_ids: HashSet<i32>,
    /// Enemy-side companion skill/buff ids the rule chain emits at
    /// `behaviorTarget == 206` (enemy). Discovered by walking
    /// defender passives that invoke (`50008`/`60225`) battle-rule
    /// skills and collecting the `AddBuff` targets those invoked
    /// skills emit at the enemy side.
    enemy_companion_act_ids: HashSet<i32>,
}

/// Build the active fight's battle-rule cycle id sets. Replaces the
/// old hardcoded `BOSS_CYCLE_ACT_ID = 530000151` /
/// `ENEMY_CYCLE_DEL_ACT_ID = 530000412` constants — those numbers
/// are stage-specific (battle1/2/3 happen to share them; another
/// stage would have different rule effects).
///
/// Two passes:
///
/// 1. **Roots**: walk `episode → battle.addition_rule → rule[id].effect`,
///    keep only ids in the battle-rule signature range
///    (`530000xxx`).
/// 2. **Enemy companion**: walk current defender passives that fall
///    in the rule-cycle range, follow `50008#<skill>` /
///    `60225#<skill>` invocations to other battle-rule skills, and
///    collect any `AddBuff` targets those invocations emit with
///    `behaviorTarget == 206` (enemy side). Excludes ids already
///    in the root family.
///
/// Internally the function also builds a transitive `AddBuff`
/// closure of the roots (used to filter the enemy-companion search
/// to relevant invocations only); only ids in the rule-cycle
/// signature range are followed, so the closure stays bounded.
fn discover_battle_rule_cycle_act_ids(fight: &Fight) -> BattleRuleCycleActIds {
    let cfg = configs::get();
    let root_act_ids = collect_battle_rule_root_skills(fight);
    if root_act_ids.is_empty() {
        return BattleRuleCycleActIds::default();
    }

    let mut queue: VecDeque<i32> = root_act_ids.iter().copied().collect();
    let mut root_family_act_ids = root_act_ids.clone();

    while let Some(skill_id) = queue.pop_front() {
        let effect_id = resolve_skill_effect_id(skill_id);
        let Some(skill) = cfg.skill_effect.iter().find(|row| row.id == effect_id) else {
            continue;
        };
        for (behavior, _) in skill_behavior_slots(skill) {
            for buff_id in parse_add_buff_ids(behavior) {
                if !is_battle_rule_cycle_id(buff_id) || !root_family_act_ids.insert(buff_id) {
                    continue;
                }
                queue.push_back(buff_id);
            }
        }
    }

    BattleRuleCycleActIds {
        root_act_ids,
        enemy_companion_act_ids: discover_enemy_rule_companion_act_ids(fight, &root_family_act_ids),
    }
}

fn collect_battle_rule_root_skills(fight: &Fight) -> HashSet<i32> {
    let cfg = configs::get();
    let episode_id = fight.episode_id.unwrap_or(0);
    let Some(battle_id) = cfg
        .episode
        .iter()
        .find(|episode| episode.id == episode_id)
        .map(|episode| episode.battle_id)
    else {
        return HashSet::new();
    };
    let Some(battle) = cfg.battle.iter().find(|row| row.id == battle_id) else {
        return HashSet::new();
    };
    if battle.addition_rule.is_empty() {
        return HashSet::new();
    }

    let mut out = HashSet::new();
    for entry in battle.addition_rule.split('|') {
        let mut parts = entry.split('#');
        let Some(prefix) = parts.next().and_then(|value| value.parse::<i32>().ok()) else {
            continue;
        };
        let Some(rule_id) = parts.next().and_then(|value| value.parse::<i32>().ok()) else {
            continue;
        };
        if !(1..=3).contains(&prefix) {
            continue;
        }
        let Some(rule) = cfg.rule.iter().find(|row| row.id == rule_id) else {
            continue;
        };
        let sid = rule.effect.parse::<i32>().ok().unwrap_or(0);
        if sid > 0 && is_battle_rule_cycle_id(sid) {
            out.insert(sid);
        }
    }

    out
}

fn discover_enemy_rule_companion_act_ids(
    fight: &Fight,
    root_family_act_ids: &HashSet<i32>,
) -> HashSet<i32> {
    let cfg = configs::get();
    let mut out = HashSet::new();
    let Some(defender) = fight.defender.as_ref() else {
        return out;
    };

    for entity in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
        if entity.uid.unwrap_or(0) >= 0 {
            continue;
        }
        for passive_id in &entity.passive_skill {
            if !is_battle_rule_cycle_id(*passive_id) {
                continue;
            }

            let effect_id = resolve_skill_effect_id(*passive_id);
            let Some(skill) = cfg.skill_effect.iter().find(|row| row.id == effect_id) else {
                continue;
            };

            for (behavior, _) in skill_behavior_slots(skill) {
                for invoked_id in parse_invoked_rule_skill_ids(behavior) {
                    let invoked_effect_id = resolve_skill_effect_id(invoked_id);
                    let Some(invoked) = cfg
                        .skill_effect
                        .iter()
                        .find(|row| row.id == invoked_effect_id)
                    else {
                        continue;
                    };

                    let mut emits_enemy_add_buff = false;
                    for (invoked_behavior, target) in skill_behavior_slots(invoked) {
                        if parse_behavior_target(target) != 206 {
                            continue;
                        }
                        for buff_id in parse_add_buff_ids(invoked_behavior) {
                            if !is_battle_rule_cycle_id(buff_id) {
                                continue;
                            }
                            if root_family_act_ids.contains(&buff_id) {
                                continue;
                            }
                            emits_enemy_add_buff = true;
                            out.insert(buff_id);
                        }
                    }

                    if emits_enemy_add_buff {
                        out.insert(invoked_id);
                    }
                }
            }
        }
    }

    out
}

fn skill_behavior_slots(skill: &SkillEffect) -> [(&str, &str); 20] {
    [
        (skill.behavior1.as_str(), skill.behavior_target1.as_str()),
        (skill.behavior2.as_str(), skill.behavior_target2.as_str()),
        (skill.behavior3.as_str(), skill.behavior_target3.as_str()),
        (skill.behavior4.as_str(), skill.behavior_target4.as_str()),
        (skill.behavior5.as_str(), skill.behavior_target5.as_str()),
        (skill.behavior6.as_str(), skill.behavior_target6.as_str()),
        (skill.behavior7.as_str(), skill.behavior_target7.as_str()),
        (skill.behavior8.as_str(), skill.behavior_target8.as_str()),
        (skill.behavior9.as_str(), skill.behavior_target9.as_str()),
        (skill.behavior10.as_str(), skill.behavior_target10.as_str()),
        (skill.behavior11.as_str(), skill.behavior_target11.as_str()),
        (skill.behavior12.as_str(), skill.behavior_target12.as_str()),
        (skill.behavior13.as_str(), skill.behavior_target13.as_str()),
        (skill.behavior14.as_str(), skill.behavior_target14.as_str()),
        (skill.behavior15.as_str(), skill.behavior_target15.as_str()),
        (skill.behavior16.as_str(), skill.behavior_target16.as_str()),
        (skill.behavior17.as_str(), skill.behavior_target17.as_str()),
        (skill.behavior18.as_str(), skill.behavior_target18.as_str()),
        (skill.behavior19.as_str(), skill.behavior_target19.as_str()),
        (skill.behavior20.as_str(), skill.behavior_target20.as_str()),
    ]
}

fn parse_add_buff_ids(behavior: &str) -> Vec<i32> {
    let mut out = Vec::new();
    if behavior.is_empty() {
        return out;
    }

    for entry in behavior.split('|') {
        let mut parts = entry.split('#');
        let behavior_id = parts
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(0);
        if behavior_id != 1 {
            continue;
        }
        let buff_id = parts
            .next()
            .and_then(|value| value.trim().parse::<i32>().ok())
            .unwrap_or(0);
        if buff_id > 0 {
            out.push(buff_id);
        }
    }

    out
}

fn parse_invoked_rule_skill_ids(behavior: &str) -> Vec<i32> {
    let mut out = Vec::new();
    if behavior.is_empty() {
        return out;
    }

    for entry in behavior.split('|') {
        if let Some(rest) = entry.strip_prefix("50008#") {
            let head = rest.split('#').next().unwrap_or("");
            let skill_id = head.trim().parse::<i32>().ok().unwrap_or(0);
            if is_battle_rule_cycle_id(skill_id) {
                out.push(skill_id);
            }
            continue;
        }

        let Some(rest) = entry.strip_prefix("60225#") else {
            continue;
        };
        for chunk in rest.split('&') {
            let head = chunk.split(':').next().unwrap_or("");
            let skill_id = head.trim().parse::<i32>().ok().unwrap_or(0);
            if is_battle_rule_cycle_id(skill_id) {
                out.push(skill_id);
            }
        }
    }

    out
}

fn parse_behavior_target(raw: &str) -> i32 {
    raw.trim().parse::<i32>().ok().unwrap_or(0)
}

fn is_battle_rule_cycle_id(id: i32) -> bool {
    (BATTLE_RULE_CYCLE_ID_MIN..BATTLE_RULE_CYCLE_ID_MAX_EXCLUSIVE).contains(&id)
}
