//! Round-end bundling: fold loose FightSteps into the host wrapper
//! of the hero that owns them. Hero modules describe their wrapper
//! via `RoundEndBundleSpec`; this module owns the generic walk plus
//! the claim-rank arbitration that picks a single owner when more
//! than one bundle could absorb a step.
//!
//! The classic example is Semmelweis's "Blood Domain" wrapper and
//! Nautika's psychube channel wrapper — both fire at round-end and
//! LIVE-side absorb side data emitted by their owner. When both
//! wrappers are present in the same round, each loose step lands
//! in whichever wrapper's claim rank is lowest.

use std::collections::{HashMap, HashSet};

use config::configs;
use sonettobuf::{ActEffect, Fight, FightStep, fight_step};

use crate::state::battle::{
    fight_step::wrap_step, hero::HeroId, utils::find_uid_by_hero_id,
};

/// Hero-side description of a round-end bundle. `host_skill_ids`
/// returns the runtime act_ids that mark a step as the hero's host
/// wrapper (post-Euphoria swaps included). `claim_rank` decides
/// whether this hero wants to absorb a non-host step — lower ranks
/// win across heroes; `None` means "I don't claim this step".
#[derive(Clone, Copy)]
pub struct RoundEndBundleSpec {
    pub owner: HeroId,
    pub host_skill_ids: fn() -> &'static HashSet<i32>,
    pub claim_rank: fn(&Fight, &FightStep, &StepOwnership) -> Option<u16>,
}

/// Per-step ownership signals collected once for every hero in the
/// active spec list. Each hero's `claim_rank` reads its own slot
/// and decides where to land in the priority cascade.
#[derive(Default)]
pub struct StepOwnership {
    by_hero: HashMap<HeroId, HeroStepSignals>,
}

impl StepOwnership {
    pub fn hero(&self, hero: HeroId) -> HeroStepSignals {
        self.by_hero.get(&hero).copied().unwrap_or_default()
    }
}

#[derive(Default, Clone, Copy)]
pub struct HeroStepSignals {
    pub from: bool,
    pub to: bool,
    pub targets: bool,
    pub skill_from: bool,
}

/// Walk `steps`, find each spec's host wrapper, and fold every
/// loose step the specs claim into the lowest-ranked owner. Steps
/// no spec claims stay where they are.
pub fn fold(fight: &Fight, steps: &mut Vec<FightStep>, specs: &[&RoundEndBundleSpec]) {
    let host_indices: Vec<Option<usize>> = specs
        .iter()
        .map(|spec| find_host_index(steps, (spec.host_skill_ids)()))
        .collect();

    if host_indices.iter().all(Option::is_none) {
        return;
    }

    let uid_by_hero = collect_uids(fight, specs);
    let mut embeds: Vec<Vec<ActEffect>> = (0..specs.len()).map(|_| Vec::new()).collect();
    let mut remove_indices = Vec::new();

    for idx in 0..steps.len() {
        if host_indices.iter().any(|h| *h == Some(idx)) {
            continue;
        }

        let ownership = inspect(&steps[idx], &uid_by_hero);

        let mut best: Option<(usize, u16)> = None;
        for (spec_idx, spec) in specs.iter().enumerate() {
            if host_indices[spec_idx].is_none() {
                continue;
            }
            let Some(rank) = (spec.claim_rank)(fight, &steps[idx], &ownership) else {
                continue;
            };
            if best.is_none_or(|(_, current)| rank < current) {
                best = Some((spec_idx, rank));
            }
        }

        if let Some((spec_idx, _)) = best {
            embeds[spec_idx].push(wrap_step(steps[idx].clone()));
            remove_indices.push(idx);
        }
    }

    for (spec_idx, host_idx) in host_indices.iter().enumerate() {
        if let Some(idx) = host_idx
            && !embeds[spec_idx].is_empty()
        {
            steps[*idx]
                .act_effect
                .extend(std::mem::take(&mut embeds[spec_idx]));
        }
    }

    if remove_indices.is_empty() {
        return;
    }
    remove_indices.sort_unstable();
    remove_indices.dedup();
    for idx in remove_indices.into_iter().rev() {
        steps.remove(idx);
    }
}

/// Read the `act_id` of the host wrapper inside an effect-typed
/// step. The host shape is: an `Effect` step whose first
/// `act_effect` carries `effect_type=162` with a nested `Effect`
/// fight_step — the inner step's `act_id` is the host skill id.
fn host_act_id(step: &FightStep) -> Option<i32> {
    if step.act_type != Some(fight_step::ActType::Effect as i32) {
        return None;
    }
    let wrapped = step.act_effect.first()?;
    if wrapped.effect_type != Some(162) {
        return None;
    }
    let host = wrapped.fight_step.as_ref()?;
    if host.act_type != Some(fight_step::ActType::Effect as i32) {
        return None;
    }
    host.act_id
}

fn find_host_index(steps: &[FightStep], hosts: &HashSet<i32>) -> Option<usize> {
    steps
        .iter()
        .position(|step| host_act_id(step).is_some_and(|act_id| hosts.contains(&act_id)))
}

fn collect_uids(fight: &Fight, specs: &[&RoundEndBundleSpec]) -> HashMap<HeroId, i64> {
    let mut out = HashMap::new();
    for spec in specs {
        if let Some(uid) = find_uid_by_hero_id(fight, spec.owner.model_id()) {
            out.insert(spec.owner, uid);
        }
    }
    out
}

fn inspect(step: &FightStep, uid_by_hero: &HashMap<HeroId, i64>) -> StepOwnership {
    let mut by_hero: HashMap<HeroId, HeroStepSignals> = HashMap::new();
    collect(step, uid_by_hero, &mut by_hero);
    StepOwnership { by_hero }
}

fn collect(
    step: &FightStep,
    uid_by_hero: &HashMap<HeroId, i64>,
    out: &mut HashMap<HeroId, HeroStepSignals>,
) {
    let is_skill = step.act_type == Some(fight_step::ActType::Skill as i32);

    for (&hero, &uid) in uid_by_hero.iter() {
        let entry = out.entry(hero).or_default();
        if step.from_id == Some(uid) {
            entry.from = true;
            if is_skill {
                entry.skill_from = true;
            }
        }
        if step.to_id == Some(uid) {
            entry.to = true;
        }
    }

    for effect in &step.act_effect {
        if let Some(target) = effect.target_id {
            for (&hero, &uid) in uid_by_hero.iter() {
                if target == uid {
                    out.entry(hero).or_default().targets = true;
                }
            }
        }
        if let Some(child) = effect.fight_step.as_ref() {
            collect(child, uid_by_hero, out);
        }
    }
}

/// Helper for hero modules to discover their own host skill ids by
/// feature pattern — a buff qualifies if its `features` list carries
/// every `required_act_type` named (matched against `buff_act.type`).
pub fn discover_buff_ids_with_acts(required_act_types: &[&str]) -> HashSet<i32> {
    let cfg = configs::get();
    let buff_act_types: HashMap<i32, &str> = cfg
        .buff_act
        .iter()
        .map(|a| (a.id, a.r#type.as_str()))
        .collect();
    let mut out = HashSet::new();
    for buff in cfg.skill_buff.iter() {
        if buff.features.is_empty() {
            continue;
        }
        let feature_acts: HashSet<&str> = buff
            .features
            .split('|')
            .filter_map(|entry| {
                let act_id: i32 = entry.split('#').next()?.trim().parse().ok()?;
                buff_act_types.get(&act_id).copied()
            })
            .collect();
        if required_act_types
            .iter()
            .all(|needed| feature_acts.contains(needed))
        {
            out.insert(buff.id);
        }
    }
    out
}
