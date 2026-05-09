//! Nautika — Faith resource (`exPointType=1`, max=8) replaces Moxie
//! for her kit. Whenever bloodtithe accumulates and Nautika is the
//! losing side or the involved entity, the engine also emits a Faith
//! delta (effect type 111 = `ExPointChange`). The shared bloodtithe
//! path calls into the helpers here so the Nautika-specific branches
//! don't have to spell out model-id literals or duplicate the side
//! lookup.
//!
//! Form-shift handling: her `character.skill` / `character.exSkill`
//! rows are stale (the `character_rank_replace.json` overlay carries
//! the actual in-engine ids), and the EX itself is only present on
//! upgraded `skill_ex_level` rows. `resolve_ex` and `resolve_form_shift_group`
//! own that special-case lookup so `entity::skill` doesn't have to
//! spell out a Nautika-specific branch.

use std::collections::{HashMap, HashSet};

use config::configs;
use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::{
    fight_step::ActEffectBuilder,
    hero::HeroId,
    round::round_end_bundling::{RoundEndBundleSpec, StepOwnership, discover_buff_ids_with_acts},
    skill::cache::resolve_skill_effect_id,
};

pub fn is_nautika(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Nautika.model_id())
}

/// Faith +1 emission targeting Nautika directly. Used by the
/// bloodtithe consume paths where Nautika herself takes the HP loss
/// — a fixed +1 alongside the bloodpool delta.
pub fn faith_gain_one(target: i64) -> ActEffect {
    ActEffectBuilder::ex_point_change(target, 1)
}

/// Faith gain by the bloodtithe-gain amount. Used when any ally loses
/// HP and Nautika banks Faith equal to the resulting bloodpool delta.
pub fn faith_gain_amount(nautika_uid: i64, gained: i32) -> ActEffect {
    ActEffectBuilder::ex_point_change(nautika_uid, gained)
}

/// Locate Nautika on the requested side. Returns `None` if she isn't
/// on that team or isn't in the fight at all.
pub fn find_uid(fight: &Fight, team_type: i32) -> Option<i64> {
    let side = if team_type == 1 {
        fight.attacker.as_ref()
    } else {
        fight.defender.as_ref()
    };
    side?
        .entitys
        .iter()
        .find(|e| e.model_id == Some(HeroId::Nautika.model_id()))
        .and_then(|e| e.uid)
}

/// Skill ids that act as Nautika's channel-cast host wrappers — any
/// buff whose features carry the `NuoDiKaCastChannel` buff_act type
/// (`1006`). The act type is named after her romanization, so the
/// set is signature-locked to her in the data tables today. Both
/// the round-end bundle merger (here) and the round-end emit
/// cleanup in `mechanics::nautika` read from this set, so the
/// channel-host detection has one source of truth.
pub static CHANNEL_HOST_SKILL_IDS: Lazy<HashSet<i32>> =
    Lazy::new(|| discover_buff_ids_with_acts(&["NuoDiKaCastChannel"]));

fn round_end_bundle_hosts() -> &'static HashSet<i32> {
    Lazy::force(&CHANNEL_HOST_SKILL_IDS)
}

/// Nautika's claim ranks slot above and below Semmelweis's two
/// claim points: her ult-skill rank (10) wins outright, and her
/// general from/to rank (30) lands between Semmelweis's skill-from
/// (20) and her general from/to/targets (40). The trailing
/// targets-only rank (50) is the lowest in the cascade.
fn claim_round_end_bundle_step(
    _fight: &Fight,
    _step: &FightStep,
    ownership: &StepOwnership,
) -> Option<u16> {
    let me = ownership.hero(HeroId::Nautika);
    if me.skill_from {
        Some(10)
    } else if me.from || me.to {
        Some(30)
    } else if me.targets {
        Some(50)
    } else {
        None
    }
}

pub static ROUND_END_BUNDLE: RoundEndBundleSpec = RoundEndBundleSpec {
    owner: HeroId::Nautika,
    host_skill_ids: round_end_bundle_hosts,
    claim_rank: claim_round_end_bundle_step,
};

/// Resolve Nautika's runtime EX from the `skill_ex_level` chain when
/// her hero matches. Her `character.exSkill` row is stale — the
/// actual EX lives in `skill_ex_level` (Insight Lv.1 / Lv.2 / etc.).
/// Returns `None` for non-Nautika hero ids; the caller falls back to
/// the standard `character.exSkill` lookup.
///
/// `destiny`, when present, is the cumulative Euphoria swap map; the
/// resolved EX is routed through it before being returned.
pub fn resolve_ex(
    hero_id: i32,
    ex_skill_level: i32,
    destiny: Option<&HashMap<i32, i32>>,
) -> Option<i32> {
    if hero_id != HeroId::Nautika.model_id() {
        return None;
    }
    let game = configs::get();
    let mut ex = game
        .skill_ex_level
        .iter()
        .find(|s| s.hero_id == hero_id && s.skill_level == ex_skill_level)
        .map(|s| s.skill_ex)
        .unwrap_or(0);
    if ex == 0 {
        ex = game
            .skill_ex_level
            .iter()
            .find(|s| s.hero_id == hero_id && s.skill_level == 1)
            .map(|s| s.skill_ex)
            .unwrap_or(0);
    }
    if let Some(map) = destiny
        && let Some(replaced) = map.get(&ex)
    {
        ex = *replaced;
    }
    Some(ex)
}

/// Resolve Nautika's slot skill list from the `skill_ex_level`
/// form-shift triplets when her hero and the requested group match.
/// Each Insight tier overlays a richer form-shift block on slot 2
/// (primary form + alt form 1 + alt form 2 triplets); the overlay
/// is parsed by hero-type and each id routed through the
/// skill_effect mapping so cards carry executable ids.
///
/// Returns `None` when the hero isn't Nautika or when no overlay
/// row is present yet — callers fall through to the
/// `character.skill` lookup.
pub fn resolve_form_shift_group(
    hero_id: i32,
    group: i32,
    ex_level: i32,
    hero_type: i32,
) -> Option<Vec<i32>> {
    if hero_id != HeroId::Nautika.model_id() {
        return None;
    }
    let raw = lookup_ex_group(hero_id, group, ex_level);
    if raw.is_empty() {
        return None;
    }
    Some(
        parse_ex_string(raw, hero_type)
            .into_iter()
            .map(resolve_skill_effect_id)
            .collect(),
    )
}

fn lookup_ex_group(hero_id: i32, group: i32, ex_level: i32) -> &'static str {
    let game = configs::get();
    for lvl in (1..=ex_level).rev() {
        if let Some(ex) = game
            .skill_ex_level
            .iter()
            .find(|s| s.hero_id == hero_id && s.skill_level == lvl)
        {
            match group {
                1 if !ex.skill_group1.is_empty() => return &ex.skill_group1,
                2 if !ex.skill_group2.is_empty() => return &ex.skill_group2,
                _ => {}
            }
        }
    }
    ""
}

fn parse_ex_string(s: &str, hero_type: i32) -> Vec<i32> {
    if s.contains(',') {
        let sets: Vec<&str> = s.split(',').collect();
        let idx = if hero_type < 1 || hero_type as usize > sets.len() {
            0
        } else {
            (hero_type - 1) as usize
        };
        sets.get(idx)
            .into_iter()
            .flat_map(|v| v.split('|'))
            .filter_map(|v| v.parse().ok())
            .collect()
    } else {
        s.split('|').filter_map(|v| v.parse().ok()).collect()
    }
}
