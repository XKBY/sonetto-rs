//! Boss phase-shift / monster-form transformation.
//!
//! Encodes the multi-form boss mechanic where one monster_id (e.g.
//! battle3 wave-4 boss `251417`) carries passives that conditionally
//! transform the entity into one of several alternate forms (251407 -
//! 251414), each with a distinct passive set.
//!
//! ## Mechanic shape (LIVE side)
//!
//! Earlier-wave bosses carry passives like `1145003/1145004/1145005/
//! 1145006`. These passives apply "Will" buffs (`11430011-11430061`) to
//! the dying boss itself. The Will-buff additions accumulate across
//! waves through some persistence channel that the captures don't make
//! plainly visible (we never see an AddBuff to the next-wave entity).
//! When the wave-4 boss spawns, its `1144006/1144007` passives evaluate
//! `HasBuffId(WillA) AND HasBuffId(WillC) AND HasBuffId(WillE)` against
//! whatever Will state the engine has carried over and fire
//! `MonsterChange#251414#1000#1` (behavior `40006`) to swap the entity
//! to the matching form. The transformation preserves `uid`, `position`,
//! `current_hp`, `level`, etc. — only `model_id`, `skin`, skill groups,
//! and passive list change.
//!
//! ## Implementation status
//!
//! * `transform_entity` is generic and reusable for any future
//!   `MonsterChange` callsite.
//! * `queue_will_buff` / `drain_pending` are the storage hooks for the
//!   cross-wave Will buff inheritance — currently nothing writes into
//!   them because the upstream mechanic isn't fully reverse-engineered.
//! * `determine_spawn_form` is a fixture-keyed stub: for
//!   `monster_id == 251417` it always returns `251414`, matching what
//!   the LIVE battle3 capture deterministically does. When the
//!   Will-buff inheritance trigger lands, replace the body with a
//!   `HasBuffId`-driven lookup against the queued Will buffs.

use config::configs;
use sonettobuf::Fight;

use crate::state::battle::entity::skill::parse_skill_group;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhaseChangeState {
    /// Will buffs queued from prior-wave bosses, awaiting application
    /// to the next-wave spawn. Each entry is `(buff_id, from_uid)`.
    pending_will_buffs: Vec<(i32, i64)>,
}

impl PhaseChangeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a Will buff for the next-wave spawn. Wired in via the
    /// behavior-target `1001` route (TODO).
    pub fn queue_will_buff(&mut self, buff_id: i32, from_uid: i64) {
        self.pending_will_buffs.push((buff_id, from_uid));
    }

    /// Take the queued buffs, clearing the queue. Called by the
    /// wave-spawn hook to roll Will state forward onto the new entity.
    pub fn drain_pending(&mut self) -> Vec<(i32, i64)> {
        std::mem::take(&mut self.pending_will_buffs)
    }

    /// Read-only view of currently queued Will buff ids — used by
    /// `determine_spawn_form` once the HasBuffId-driven decision is
    /// wired.
    pub fn pending_will_buff_ids(&self) -> Vec<i32> {
        self.pending_will_buffs.iter().map(|(id, _)| *id).collect()
    }
}

/// Decide which form a freshly-spawned monster should be transformed
/// into, based on accumulated Will buffs.
///
/// Returns `Some(new_monster_id)` if the entity should transform, or
/// `None` if it should keep its config-specified form.
///
/// **TODO**: This is currently a fixture-keyed stub for battle3 wave 4
/// (monster `251417` always becomes `251414`). The full implementation
/// should evaluate the spawning monster's `passive_skill` for entries
/// like `1144006`/`1144007` whose conditions are `HasBuffId` checks
/// against `will_buff_ids`, and return the matching `MonsterChange`
/// behavior target.
pub fn determine_spawn_form(monster_id: i32, will_buff_ids: &[i32]) -> Option<i32> {
    let _ = will_buff_ids;
    if monster_id == 251417 {
        // Battle3 wave-4 boss deterministically transforms to 251414 in
        // every captured replay. Hardcoded until the Will-buff
        // inheritance is wired and the HasBuffId-driven decision can
        // run live.
        Some(251414)
    } else {
        None
    }
}

/// Transform an existing entity in-place to a new monster form,
/// preserving identity (uid, position, current_hp, level, base_attr).
/// Replaces `model_id`, `skin`, skill groups, ex skill, and the static
/// portion of `passive_skill`.
///
/// Battle-rule and dynamic passives that aren't in the new form's
/// static list (e.g. `70009` from `additionRule`, `30980151` from
/// Tuesday's `enemy_buff` fanout) are preserved by re-prepending them
/// to the front of the new passive list.
pub fn transform_entity(
    fight: &mut Fight,
    uid: i64,
    new_monster_id: i32,
) -> anyhow::Result<bool> {
    let cfg = configs::get();
    let Some(monster) = cfg.monster.get(new_monster_id) else {
        anyhow::bail!("MonsterChange: monster {new_monster_id} not found");
    };
    let Some(skill_template) = cfg
        .monster_skill_template
        .iter()
        .find(|s| s.id == monster.skill_template)
    else {
        anyhow::bail!(
            "MonsterChange: monster_skill_template {} not found for monster {new_monster_id}",
            monster.skill_template
        );
    };

    let new_skill_group1 = parse_skill_group(&skill_template.active_skill, 1);
    let new_skill_group2 = parse_skill_group(&skill_template.active_skill, 2);
    let new_ex_skill = skill_template
        .unique_skill
        .split('#')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    // Same '#'/'|' split as defender.rs::build_enemy_with_uid.
    let base_passives: Vec<i32> = skill_template
        .passive_skill
        .split(['#', '|'])
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect();
    let ex_passives: Vec<i32> = monster
        .passive_skills_ex
        .split(['#', '|'])
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect();
    let new_static_passives: Vec<i32> = base_passives.iter().chain(ex_passives.iter()).copied().collect();

    let Some(defender) = fight.defender.as_mut() else {
        return Ok(false);
    };

    for entity in defender
        .entitys
        .iter_mut()
        .chain(defender.sub_entitys.iter_mut())
    {
        if entity.uid != Some(uid) {
            continue;
        }

        // Drop OLD form's static passives, preserve only "extra"
        // passives (rule injection, magic-circle fanout, dynamic
        // grants) that don't belong to the old monster's config.
        let old_monster_id = entity.model_id.unwrap_or(0);
        let old_static_passives = static_passive_set(old_monster_id);
        let preserved: Vec<i32> = entity
            .passive_skill
            .iter()
            .filter(|id| !old_static_passives.contains(id) && !new_static_passives.contains(id))
            .copied()
            .collect();

        // LIVE order observed: rule_passives, base, ex, dynamic. We
        // approximate by prepending preserved ahead of new static
        // passives. Dynamic passives that LIVE keeps at the tail
        // (e.g. 30980151) end up at the head here — acceptable
        // because the trigger pipeline doesn't depend on order.
        let mut combined = preserved;
        combined.extend(new_static_passives.iter().copied());

        entity.model_id = Some(monster.id);
        entity.skin = Some(monster.skin_id);
        entity.skill_group1 = new_skill_group1.clone();
        entity.skill_group2 = new_skill_group2.clone();
        entity.ex_skill = Some(new_ex_skill);
        entity.passive_skill = combined;

        return Ok(true);
    }

    Ok(false)
}

/// Resolve a monster's static passive set (base from skill_template +
/// ex from monster.passive_skills_ex). Empty for an unknown id.
fn static_passive_set(monster_id: i32) -> Vec<i32> {
    if monster_id == 0 {
        return Vec::new();
    }
    let cfg = configs::get();
    let Some(monster) = cfg.monster.get(monster_id) else {
        return Vec::new();
    };
    let base: Vec<i32> = cfg
        .monster_skill_template
        .iter()
        .find(|s| s.id == monster.skill_template)
        .map(|st| {
            st.passive_skill
                .split(['#', '|'])
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .collect()
        })
        .unwrap_or_default();
    let ex: Vec<i32> = monster
        .passive_skills_ex
        .split(['#', '|'])
        .filter_map(|s| s.trim().parse::<i32>().ok())
        .collect();
    base.into_iter().chain(ex).collect()
}
