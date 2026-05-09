//! Semmelweis — `And So It Rises Again` (Tier IV swap to
//! `The Red from a Thousand Moons`) and the Moxie-from-bloodpool
//! Insight chain. The orphan passives the data tables don't list
//! (`308801911`, `308801921`, `308802111`) flow through `destiny.rs`;
//! this module owns the hero-specific rules her shared kit calls
//! back into.

use std::collections::HashSet;

use once_cell::sync::Lazy;
use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::{
    entity::destiny::Destiny,
    fight_step::ActEffectBuilder,
    hero::HeroId,
    heroes::rubuska,
    mechanics::bloodtithe::is_bloodtithe_enabled,
    round::round_end_bundling::{RoundEndBundleSpec, StepOwnership, discover_buff_ids_with_acts},
    skill::targets::alive_allies,
    types::effects::EffectType,

};

/// Insight-Lv.1 base id of `And So It Rises Again`. The Lv.0 base
/// (`30880131`) lives in `character.exSkill`; `30880132` is the
/// Insight-Lv.1 upgrade Tier IV swaps to the bank-distribution
/// version. Both ids are config truth from `skill_ex_level`.
const ULT_VARIANT_2_BASE: i32 = 30880132;
const TIER_IV: i32 = 4;

/// Tier-IV bank-distribution variant of Semmelweis's ult. Resolved
/// once at startup by walking the Euphoria swap chain rather than
/// hardcoded — `Destiny::resolve_skill_id` turns `30880132` into the
/// in-engine post-swap id when Tier IV is unlocked.
pub static TIER_IV_ULT_SKILL_ID: Lazy<i32> = Lazy::new(|| {
    let facets_id = Destiny::facets_id_for_hero(HeroId::Semmelweis.model_id())
        .expect("Semmelweis missing from character_destiny");
    Destiny::resolve_skill_id(facets_id, TIER_IV, ULT_VARIANT_2_BASE)
});

/// Replay-time bloodpool gain for one of the four visible 335 packets
/// the Tier IV ult body emits. The engine reads each ally's kit
/// rather than enumerating heroes by id — anyone whose passives
/// grant a `BloodPoolTag`-bearing buff (per
/// `bloodtithe::is_bloodtithe_enabled`) takes a +1 packet, and the
/// Shadow Cloak holder also picks up one extra packet for the
/// bloodtithe share Rubuska's cloak banks LIVE-side.
pub fn ult_manual_gain(model_id: Option<i32>) -> i32 {
    let Some(hero_id) = model_id else {
        return 0;
    };
    let mut gain = 0;
    if is_bloodtithe_enabled(hero_id) {
        gain += 1;
    }
    if rubuska::is_rubuska(model_id) {
        gain += 1;
    }
    gain
}

/// Round-end bundle host skill ids for Semmelweis's `Blood Domain`
/// wrapper family. Every variant of `And So It Rises Again` (base
/// EX, Insight upgrade, Tier III/IV Euphoria swaps) marks itself
/// with the `UseSkillToEnemy` + `ControlTeamInjuryCountRound` act
/// pair in `skill_buff::features`; we discover them by feature
/// pattern so the cascade survives swap-chain growth.
static ROUND_END_BUNDLE_HOSTS: Lazy<HashSet<i32>> =
    Lazy::new(|| discover_buff_ids_with_acts(&["UseSkillToEnemy", "ControlTeamInjuryCountRound"]));

fn round_end_bundle_hosts() -> &'static HashSet<i32> {
    Lazy::force(&ROUND_END_BUNDLE_HOSTS)
}

/// Semmelweis fits the round-end cascade between Nautika's two
/// general claims: her ult-skill rank wins below Nautika's `from/to`
/// (rank 30) but her general from/to/targets falls behind it.
fn claim_round_end_bundle_step(
    _fight: &Fight,
    _step: &FightStep,
    ownership: &StepOwnership,
) -> Option<u16> {
    let me = ownership.hero(HeroId::Semmelweis);
    if me.skill_from {
        Some(20)
    } else if me.from || me.to || me.targets {
        Some(40)
    } else {
        None
    }
}

pub static ROUND_END_BUNDLE: RoundEndBundleSpec = RoundEndBundleSpec {
    owner: HeroId::Semmelweis,
    host_skill_ids: round_end_bundle_hosts,
    claim_rank: claim_round_end_bundle_step,
};

/// Blood Domain (`100051`) is Semmelweis's only magic circle. Its
/// `selfBuff` (`308801312`) carries the `CureUpByLostHp` feature,
/// which means the buff fans out to every alive ally rather than
/// landing on the caster alone, paired with a `CureUpByLostHp`
/// marker per ally. The generic `mechanics::magic_circle`
/// orchestrator detects the feature and dispatches into this
/// helper — the dispatch is feature-driven, the body is
/// hero-owned because she's the only hero who runs this aura
/// shape today.
pub fn expand_blood_domain_self_buff_aura(
    fight: &Fight,
    caster_uid: i64,
    buff_id: i32,
) -> Vec<ActEffect> {
    let mut out = Vec::with_capacity(alive_allies(fight, caster_uid).len() * 2);
    for ally_uid in alive_allies(fight, caster_uid) {
        out.push(crate::state::battle::fight_step::ActEffectBuilder::buff_add(caster_uid, ally_uid, buff_id, 1));
        out.push(
            ActEffectBuilder::new(EffectType::CureUpByLostHp as i32, ally_uid)
                .effect_num(0)
                .build(),
        );
    }
    out
}



