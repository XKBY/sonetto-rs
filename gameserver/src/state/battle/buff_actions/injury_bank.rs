//! Handler for buff_act 770 InjuryBank — Kakania Empathy storage / Insight III
//! reactive params (storage cap, threshold, heal, bounce-skill linkage).
//!
//! Feature shape on Kakania's Empathy buffs:
//! `770#<attr_id>#<storage_cap_permille>#<bounce_skill_id>#<storage_threshold_permille>#<heal_permille>#…`
//!
//! Examples in the data:
//! - `30800141`: `770#101#200#30800161#30#100#100` (canonical Insight I)
//! - `30800142`: `770#101#200#30800162#20#100#100` (Insight II rank)
//! - `30800143`: `770#101#300#30800162#20#100#150` (Tier-IV destiny variant)
//!
//! `attr_id 101` resolves to MaxHP, so cap / threshold / heal are all applied
//! as `target_max_hp × permille / 1000` at the call sites.

#[allow(dead_code)]
pub const BUFF_ACT_ID: i32 = 770;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct InjuryBankParams {
    /// Attribute id used to scale cap / threshold / heal — typically `101` (MaxHP).
    pub attr_id: i32,
    /// Storage cap as permille of `attr_id` (e.g. `200` = 20% of MaxHP).
    pub storage_cap_permille: i32,
    /// Skill id fired on Insight III heal-trigger crossings (each
    /// `storage_threshold_permille` step). Carries an
    /// `OriginDamageFromInjuryBankBuff`-typed behavior whose multiplier
    /// scales the bounce damage.
    pub insight_iii_bounce_skill_id: i32,
    /// Storage step (per-ally) as permille of `attr_id` — Insight III
    /// fires one `InjuryBankHeal` + one bounce per crossing.
    pub storage_threshold_permille: i32,
    /// `InjuryBankHeal` amount as permille of `attr_id`.
    pub heal_permille: i32,
}

/// Parse the InjuryBank feature payload from a specific Empathy variant.
/// Returns `None` if `buff_id` is invalid or carries no `InjuryBank` feature.
pub fn buff_get_injury_bank_params(buff_id: i32) -> Option<InjuryBankParams> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let is_injury_bank = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == "InjuryBank")
            .unwrap_or(false);
        if !is_injury_bank {
            return None;
        }
        Some(InjuryBankParams {
            attr_id: parts.get(1).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            storage_cap_permille: parts.get(2).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
            insight_iii_bounce_skill_id: parts
                .get(3)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            storage_threshold_permille: parts
                .get(4)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            heal_permille: parts.get(5).and_then(|v| v.trim().parse().ok()).unwrap_or(0),
        })
    })
}
