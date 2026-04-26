use anyhow::Result;
use config::configs;
use sonettobuf::{ActEffect, Fight, MagicCircleInfo};

use super::action::{ActionCtx, BehaviorAction};
use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::skill::targets::alive_allies;
use crate::state::battle::types::behavior::BehaviorType;
use crate::state::battle::types::condition::ConditionType;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::{buff_add, for_each_buff_feature_chain};

/// Misc action — handler for the behavior variants that are
/// currently no-ops or simple skill-execution placeholders. Each
/// variant either emits nothing or logs a warning.
///
/// Variants owned (all currently emit `Ok(vec![])`):
/// * `Summon { .. }` — placeholder; no live data uses this yet.
/// * `Kill` — placeholder.
/// * `MonsterChange` — placeholder.
/// * `ShellUseSkill { .. }` — Shell-system placeholder.
/// * `ShellAssign { .. }` — Shell-system placeholder.
/// * `BeAttackedAssassinate { .. }` — placeholder.
/// * `CrystalAddCard` — placeholder.
/// * `IgnoreSkillConfigDamageRate` — flag-only behavior; the actual
///   suppression happens elsewhere in the executor.
/// * `Unknown { raw }` — log and skip.
pub(super) struct Misc;

impl BehaviorAction for Misc {
    fn execute(
        behavior: &BehaviorType,
        _ctx: &mut ActionCtx<'_, '_>,
        _condition: &ConditionType,
    ) -> Result<Vec<ActEffect>> {
        match behavior {
            BehaviorType::Summon { .. }
            | BehaviorType::Kill
            | BehaviorType::MonsterChange
            | BehaviorType::ShellUseSkill { .. }
            | BehaviorType::ShellAssign { .. }
            | BehaviorType::BeAttackedAssassinate { .. }
            | BehaviorType::CrystalAddCard
            | BehaviorType::IgnoreSkillConfigDamageRate
            | BehaviorType::MagicCircleAttr { .. } => Ok(vec![]),
            BehaviorType::Unknown { raw } => {
                tracing::warn!("Skipping unknown behavior: {}", raw);
                Ok(vec![])
            }
            _ => Ok(vec![]),
        }
    }
}

pub fn add_magic_circle(fight: &Fight, caster_uid: i64, circle_id: i32) -> Result<Vec<ActEffect>> {
    let circle = configs::get().magic_circle.get(circle_id).cloned();
    let round = circle.as_ref().map(|circle| circle.round).unwrap_or(0);
    let mut out = Vec::new();
    if let Some(buff_id) = circle
        .as_ref()
        .and_then(|circle| circle.self_buff.trim().parse::<i32>().ok())
        .filter(|id| *id > 0)
    {
        let mut has_cure_up_by_lost_hp = false;
        for_each_buff_feature_chain(buff_id, |act_type, _| {
            if act_type == "CureUpByLostHp" {
                has_cure_up_by_lost_hp = true;
            }
        });

        if has_cure_up_by_lost_hp {
            // LIVE emits paired (BuffAdd, CureUpByLostHp) packets per ally
            // when the magic circle's selfBuff carries a CureUpByLostHp
            // feature — see Semmelweis circle 100051 / buff 308801312.
            for ally_uid in alive_allies(fight, caster_uid) {
                out.push(buff_add(caster_uid, ally_uid, buff_id, 1));
                out.push(
                    ActEffectBuilder::new(EffectType::CureUpByLostHp as i32, ally_uid)
                        .effect_num(0)
                        .build(),
                );
            }
        } else {
            out.push(buff_add(caster_uid, caster_uid, buff_id, 1));
        }
    }
    out.push(
        ActEffectBuilder::new(EffectType::MagicCircleAdd as i32, caster_uid)
            .effect_num(0)
            .reserve_id(circle_id as i64)
            .magic_circle(MagicCircleInfo {
                magic_circle_id: Some(circle_id),
                round: Some(round),
                create_uid: Some(caster_uid),
                electric_level: Some(0),
                electric_progress: Some(0),
                max_electric_progress: Some(0),
            })
            .build(),
    );

    Ok(out)
}

pub fn magic_circle_attr() -> Result<Vec<ActEffect>> {
    Ok(vec![])
}
