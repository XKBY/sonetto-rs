//! Semmelweis — `And So It Rises Again` (Tier IV swap to
//! `The Red from a Thousand Moons`) and the Moxie-from-bloodpool
//! Insight chain. The orphan passives the data tables don't list
//! (`308801911`, `308801921`, `308802111`) flow through `destiny.rs`;
//! this module owns the hero-specific rules her shared kit calls
//! back into.

use once_cell::sync::Lazy;

use crate::state::battle::{
    entity::destiny::Destiny, hero::HeroId, heroes::rubuska,
    mechanics::bloodtithe::is_bloodtithe_enabled,
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
