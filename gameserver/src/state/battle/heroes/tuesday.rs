//! Tuesday — `Please, Come On In` (EX) plus the
//! `In Mother's Arms` / `The Horror's Delight` basics. Her kit
//! drives Poison / DeadlyPoison DOT ticks through the generic
//! `mechanics::dot` machinery (buff_act 803 / 844) and gates several
//! conditions on `HasBuffGroup` / `NoBuffGroup` (77208 / 78208) per
//! the Tuesday-class debuffs.
//!
//! `pick_lock_sound_enemy_target` owns the Lock-Sound Phenomenon
//! (`22100003`) circle's `enemy_buff` target picker; the generic
//! `mechanics::magic_circle` orchestrator dispatches into it when
//! the circle's creator is Tuesday.

use sonettobuf::Fight;

use crate::state::battle::{
    buff_actions::EffectContext,
    hero::HeroId,
    mechanics::dot::parse_dot_features,
    skill::targets::{alive_enemies, get_entity},
};

#[allow(dead_code)]
pub fn is_tuesday(model_id: Option<i32>) -> bool {
    model_id == Some(HeroId::Tuesday.model_id())
}

/// Lock-Sound Phenomenon (`22100003`) picks a single opposing entity
/// to receive the circle's `enemy_buff` (Lock Poison, `30980131`).
/// LIVE picks the alive opponent with the most current HP, breaking
/// ties on the fewest Poison-family stacks. Newly-spawned wave
/// entities satisfy both branches naturally (full HP, zero stacks),
/// so the buff lands on them as soon as they appear — matching
/// `31040141`'s "prioritizing targets with the most HP" rule from
/// her wider kit.
pub fn pick_lock_sound_enemy_target(
    ctx: &EffectContext<'_>,
    fight: &Fight,
    caster_uid: i64,
) -> Option<i64> {
    let buff_mgr = ctx.buff_mgr();
    alive_enemies(fight, caster_uid)
        .into_iter()
        .max_by_key(|&uid| {
            let hp = get_entity(fight, uid)
                .and_then(|e| e.current_hp)
                .unwrap_or(0);
            let poison_stacks: i32 = buff_mgr
                .get(uid)
                .iter()
                .filter(|inst| parse_dot_features(inst.buff_id).is_some())
                .map(|inst| inst.layer.max(1))
                .sum();
            (hp, -poison_stacks)
        })
}
