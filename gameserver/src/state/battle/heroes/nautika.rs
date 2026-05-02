//! Nautika — Faith resource (`exPointType=1`, max=8) replaces Moxie
//! for her kit. Whenever bloodtithe accumulates and Nautika is the
//! losing side or the involved entity, the engine also emits a Faith
//! delta (effect type 111 = `ExPointChange`). The shared bloodtithe
//! path calls into the helpers here so the Nautika-specific branches
//! don't have to spell out model-id literals or duplicate the side
//! lookup.
//!
//! Other Nautika-specific behavior (form-shift slot resolution, the
//! `skill_ex_level` lookup that backs her EX) still lives in
//! `entity/skill.rs`; that block is the next migration target.

use sonettobuf::{ActEffect, Fight};

use crate::state::battle::{fight_step::ActEffectBuilder, hero::HeroId};

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
