use sonettobuf::{ActEffect, FightStep};

use crate::state::battle::fight_step::{FightStepBuilder, wrap_step};
fn preserve_round_start_wrapper(step: &FightStep) -> bool {
    step.act_id == Some(30630171)
}

fn is_round_start_wrapper_container(effect: &ActEffect) -> bool {
    effect
        .fight_step
        .as_ref()
        .map(|step| {
            step.act_effect.iter().all(|inner| {
                inner
                    .fight_step
                    .as_ref()
                    .map(preserve_round_start_wrapper)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn build_effect_step(effects: Vec<ActEffect>) -> FightStep {
    FightStepBuilder::effect().with_many(effects).build()
}

/// Split a passive-phase result into:
/// - one flat EFFECT step containing every `effectType == 7`
/// - one wrapped EFFECT step containing everything else
///
/// This is intentionally not a shallow top-level split. If a `162` wrapper
/// contains inner `7` effects, those updates are hoisted into the flat step
/// and the wrapper is kept only for its remaining non-update inner effects.
pub fn split_updates_and_wrap_rest(steps: Vec<FightStep>) -> Vec<FightStep> {
    let mut flat_effects: Vec<ActEffect> = Vec::new();
    let mut wrapped_effects: Vec<ActEffect> = Vec::new();

    for phase_step in steps {
        let mut non_update_effects: Vec<ActEffect> = Vec::new();

        for mut effect in phase_step.act_effect {
            if effect.effect_type == Some(7) {
                flat_effects.push(effect);
                continue;
            }

            if effect.effect_type == Some(162)
                && let Some(inner_step) = effect.fight_step.as_mut()
            {
                if preserve_round_start_wrapper(inner_step) {
                    non_update_effects.push(effect);
                    continue;
                }
                let mut kept_inner: Vec<ActEffect> = Vec::new();
                for inner_effect in inner_step.act_effect.drain(..) {
                    if inner_effect.effect_type == Some(7) {
                        flat_effects.push(inner_effect);
                    } else {
                        kept_inner.push(inner_effect);
                    }
                }

                if kept_inner.is_empty() {
                    continue;
                }

                inner_step.act_effect = kept_inner;
            }

            non_update_effects.push(effect);
        }

        if !non_update_effects.is_empty() {
            wrapped_effects.push(wrap_step(build_effect_step(non_update_effects)));
        }
    }

    let mut merged = Vec::new();
    if !flat_effects.is_empty() {
        if !wrapped_effects.is_empty()
            && wrapped_effects.iter().all(is_round_start_wrapper_container)
        {
            flat_effects.extend(wrapped_effects);
            merged.push(build_effect_step(flat_effects));
            return merged;
        }
        merged.push(build_effect_step(flat_effects));
    }
    if !wrapped_effects.is_empty() {
        merged.push(build_effect_step(wrapped_effects));
    }
    merged
}
