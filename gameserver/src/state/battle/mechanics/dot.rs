//! Shared DOT feature parsing helpers.
//!
//! Round-end DOT settlement now runs through
//! `buff_actions::dispatch_stage(BuffStage::RoundEndDot, ...)`. This module
//! stays as the narrow parser seam used by other mechanics that need to
//! recognize Poison-family carrier buffs.

use crate::state::battle::types::effects::EffectType;

/// Returns `(marker_effect_type, permille)` if the buff carries a
/// poison-family DOT feature, else `None`.
///
/// Matches:
/// - `Poison` (act 803) -> `(EffectType::Poison, parts[1])`
/// - `DeadlyPoison` (act 844) -> `(EffectType::DeadlyPoison, parts[1])`
pub fn parse_dot_features(buff_id: i32) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.get(buff_id)?;
    if buff.features.is_empty() {
        return None;
    }

    for entry in buff.features.split('|') {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let act_type = cfg.buff_act.get(act_id).map(|act| act.r#type.as_str())?;
        match act_type {
            "Poison" => {
                let permille: i32 = parts.get(1)?.trim().parse().ok()?;
                if permille > 0 {
                    return Some((EffectType::Poison as i32, permille));
                }
            }
            "DeadlyPoison" => {
                let permille: i32 = parts.get(1)?.trim().parse().ok()?;
                if permille > 0 {
                    return Some((EffectType::DeadlyPoison as i32, permille));
                }
            }
            _ => continue,
        }
    }

    None
}
