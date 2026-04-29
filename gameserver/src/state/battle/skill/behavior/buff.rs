use super::super::cache::{SKILL_CACHE, resolve_skill_effect_id};
use super::super::executor::SkillExecutor;
use super::buff_helper::{has_include_type, uses_slave_uid};
use crate::state::battle::buff_actions::ban_lost_life::buff_get_ban_lost_life_floor;
use sonettobuf::{ActEffect, Fight};

use super::super::super::{
    buff::{apply_buff_effects, pre_buff_effects},
    context::buff_context::BuffContext,
    manager::{buff_mgr::next_buff_uid_for_target, fight_data_mgr::Managers},
    mechanics::Mechanics,
    types::behavior::BehaviorType,
    types::buff::{BAD_BUFF_TYPES, GOOD_BUFF_TYPES, stack_type::is_stackable},
    types::condition::ConditionType,
    utils::{buff_add_slave, buff_add_with_count, buff_del, buff_update, get_exclude_buff_effects},
};

fn with_buff_ctx<R>(
    _fight: &Fight,
    managers: &mut Managers,
    f: impl FnOnce(&mut BuffContext<'_>) -> R,
) -> R {
    let mut buff_ctx = BuffContext::new(&mut managers.buff_mgr);
    f(&mut buff_ctx)
}

fn excluded_buff_or_type_ids(buff_id: i32) -> Vec<i32> {
    let cfg = config::configs::get();
    let type_id = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|b| b.type_id)
        .unwrap_or(buff_id);
    let Some(buff_type) = cfg.skill_bufftype.iter().find(|t| t.id == type_id) else {
        return Vec::new();
    };
    if buff_type.exclude_types.is_empty() {
        return Vec::new();
    }
    buff_type
        .exclude_types
        .trim_start_matches("2#")
        .split(['，', ','])
        .filter_map(|v| v.trim().parse::<i32>().ok())
        .filter(|v| *v > 0)
        .collect()
}

fn is_poison_family(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };
    buff_cfg.features.split('|').any(|entry| {
        entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .is_some_and(|act_id| matches!(act_id, 803 | 844))
    })
}

fn should_rerun_post_add_features_on_update(buff_id: i32) -> bool {
    let cfg = config::configs::get();
    let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return false;
    };

    buff_cfg.features.split('|').any(|entry| {
        let act_id = entry
            .split('#')
            .next()
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        cfg.buff_act
            .iter()
            .find(|row| row.id == act_id)
            .map(|row| row.r#type == "AddBuffBoth")
            .unwrap_or(false)
    })
}

fn infer_enter_fight_seed_buff_for_target(
    fight: &Fight,
    target_uid: i64,
    skip_skill_id: i32,
    wanted_buff_or_type_id: i32,
) -> Option<(i32, i32)> {
    let cfg = config::configs::get();
    let entity = fight
        .attacker
        .as_ref()
        .into_iter()
        .flat_map(|a| a.entitys.iter().chain(a.sub_entitys.iter()))
        .chain(
            fight
                .defender
                .as_ref()
                .into_iter()
                .flat_map(|d| d.entitys.iter().chain(d.sub_entitys.iter())),
        )
        .find(|e| e.uid == Some(target_uid))?;

    for passive_sid in &entity.passive_skill {
        if *passive_sid <= 0 || *passive_sid == skip_skill_id {
            continue;
        }
        let effect_id = resolve_skill_effect_id(*passive_sid);
        let Some(rows) = SKILL_CACHE.get(&effect_id) else {
            continue;
        };
        for row in rows {
            let BehaviorType::AddBuff { buff_id, count } = row.behavior else {
                continue;
            };
            let matches_wanted = buff_id == wanted_buff_or_type_id
                || cfg
                    .skill_buff
                    .iter()
                    .find(|b| b.id == buff_id)
                    .map(|b| b.type_id == wanted_buff_or_type_id)
                    .unwrap_or(false);
            if !matches_wanted {
                continue;
            }
            let is_enter_fight_seed = matches!(
                row.condition,
                ConditionType::EnterFight { .. }
                    | ConditionType::EnterFightAnd(_)
                    | ConditionType::EnterFightOr(_)
            );
            if is_enter_fight_seed {
                return Some((buff_id, count.max(1)));
            }
        }
    }
    None
}

fn maybe_seed_excluded_runtime_buff(
    fight: &Fight,
    managers: &mut Managers,
    caster_uid: i64,
    target_uid: i64,
    buff_id: i32,
    skill_id: i32,
) {
    // Cross-side conversion lanes (e.g. debuff swaps) can rely on a runtime-only
    // enter-fight seed buff that is absent from incoming snapshots.
    // Seed only in cross-side context to avoid self-lane reapply inflation.
    if caster_uid == 0 || caster_uid.signum() == target_uid.signum() {
        return;
    }

    for excluded_id in excluded_buff_or_type_ids(buff_id) {
        let has_existing = with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx
                .buffs(target_uid)
                .iter()
                .any(|b| b.buff_id == excluded_id || b.type_id == excluded_id)
        });
        if has_existing {
            continue;
        }
        let Some((seed_buff_id, seed_layer)) =
            infer_enter_fight_seed_buff_for_target(fight, target_uid, skill_id, excluded_id)
        else {
            continue;
        };
        with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx.add(target_uid, seed_buff_id, target_uid, 0, seed_layer);
        });
    }
}

static NONE_CONDITION: ConditionType = ConditionType::None;

pub struct BuffApplySpec<'a> {
    pub buff_id: i32,
    pub caster_uid: i64,
    pub target: i64,
    pub count: i32,
    pub has_bloodpool: bool,
    pub skill_id: i32,
    pub condition_id: i32,
    pub condition: &'a ConditionType,
}

impl<'a> BuffApplySpec<'a> {
    pub fn new(buff_id: i32) -> Self {
        Self {
            buff_id,
            caster_uid: 0,
            target: 0,
            count: 0,
            has_bloodpool: false,
            skill_id: 0,
            condition_id: 0,
            condition: &NONE_CONDITION,
        }
    }

    pub fn caster(mut self, uid: i64) -> Self {
        self.caster_uid = uid;
        self
    }

    pub fn target(mut self, uid: i64) -> Self {
        self.target = uid;
        self
    }

    pub fn count(mut self, n: i32) -> Self {
        self.count = n;
        self
    }

    pub fn skill(mut self, id: i32) -> Self {
        self.skill_id = id;
        self
    }

    pub fn condition(mut self, id: i32, cond: &'a ConditionType) -> Self {
        self.condition_id = id;
        self.condition = cond;
        self
    }

    pub fn bloodpool(mut self, has: bool) -> Self {
        self.has_bloodpool = has;
        self
    }
}

pub fn apply(
    spec: BuffApplySpec<'_>,
    executor: &mut SkillExecutor,
    fight: &Fight,
    managers: &mut Managers,
    mechanics: &mut Mechanics,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();
    let mut existing_uid_pre = with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(spec.target)
            .iter()
            .find(|b| b.buff_id == spec.buff_id)
            .map(|b| b.uid)
    });
    if existing_uid_pre.is_none() {
        maybe_seed_excluded_runtime_buff(
            fight,
            managers,
            spec.caster_uid,
            spec.target,
            spec.buff_id,
            spec.skill_id,
        );
        existing_uid_pre = with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx
                .buffs(spec.target)
                .iter()
                .find(|b| b.buff_id == spec.buff_id)
                .map(|b| b.uid)
        });
    }
    // Live parity: wrapper reapply lane adds primary buff without upfront exclude-del.
    // Deleting here causes extra `6` effects and destabilizes downstream trigger chains.
    if existing_uid_pre.is_none() {
        let exclude = with_buff_ctx(fight, managers, |buff_ctx| {
            get_exclude_buff_effects(buff_ctx.store, spec.target, spec.buff_id)
        });
        effects.extend(exclude);
    }
    // Run pre-buff feature stage for all AddBuff sources (including equip passives),
    // to preserve live ordering for HP pre-broadcast features.
    effects.extend(pre_buff_effects(
        executor,
        fight,
        managers,
        mechanics,
        spec.caster_uid,
        spec.target,
        spec.buff_id,
        spec.condition_id,
    ));

    let cfg = config::configs::get();
    let is_per_decr_ex_point = matches!(spec.condition, ConditionType::PerDecrExPoint { .. });
    let count = if is_per_decr_ex_point && spec.count <= 0 {
        managers.ex_point_mgr.get_recent_decr_ex_point(spec.caster_uid)
    } else {
        spec.count
    };
    let buff_cfg = cfg.skill_buff.iter().find(|b| b.id == spec.buff_id);
    let has_features = buff_cfg.map(|b| !b.features.is_empty()).unwrap_or(false);
    let is_no_show = buff_cfg.map(|b| b.is_no_show == 1).unwrap_or(false);
    let is_poison_family = is_poison_family(spec.buff_id);

    let effect_count = buff_cfg.map(|b| b.effect_count).unwrap_or(0);

    // If the buff already exists on target, prefer BUFFUPDATE regardless of count/effect_count.
    // This matches live behavior for stacking/re-applying passives like 30630171 -> 30631.
    let existing_uid = with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(spec.target)
            .iter()
            .find(|b| b.buff_id == spec.buff_id)
            .map(|b| b.uid)
    });

    if let Some(existing_uid) = existing_uid {
        let rerun_post_add_features = should_rerun_post_add_features_on_update(spec.buff_id);
        let cfg_type_id = buff_cfg.map(|b| b.type_id).unwrap_or(0);
        let include_types = cfg
            .skill_bufftype
            .iter()
            .find(|t| t.id == cfg_type_id)
            .map(|t| t.include_types.clone())
            .unwrap_or_default();
        let cfg_effect_count = buff_cfg.map(|b| b.effect_count).unwrap_or(0);
        let add_count = if count > 0 {
            count
        } else if cfg_effect_count > 0 {
            cfg_effect_count
        } else {
            1
        };
        let existing_stacks = with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx
                .buffs(spec.target)
                .iter()
                .find(|b| b.buff_id == spec.buff_id)
                .map(|b| b.stacks)
                .unwrap_or(0)
        });
        let has_include_type_2 = has_include_type(&include_types, "2");
        let has_include_type_10 = has_include_type(&include_types, "10");
        let has_exclude_types = cfg
            .skill_bufftype
            .iter()
            .find(|t| t.id == cfg_type_id)
            .map(|t| !t.exclude_types.is_empty())
            .unwrap_or(false);
        let has_excluded_active = if has_exclude_types {
            let excluded_ids = excluded_buff_or_type_ids(spec.buff_id);
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx
                    .buffs(spec.target)
                    .iter()
                    .any(|b| excluded_ids.contains(&b.buff_id) || excluded_ids.contains(&b.type_id))
            })
        } else {
            false
        };
        let is_layer_stackable = is_stackable(&include_types);
        let is_stackable_buff = count > 0
            || cfg
                .skill_bufftype
                .iter()
                .find(|t| t.id == cfg_type_id)
                .map(|t| is_stackable(&t.include_types))
                .unwrap_or(false);

        // IncludeType=10 families re-apply as replacement in live:
        // BUFFDEL + BUFFADD (+ apply-buff side effects like ATTR).
        if has_include_type_10 && has_exclude_types && has_excluded_active {
            let old = with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx
                    .buffs(spec.target)
                    .iter()
                    .find(|b| b.uid == existing_uid)
                    .cloned()
            });
            if let Some(old) = old {
                effects.push(buff_del(spec.target, old.uid, old.buff_id, old.from_uid));
                with_buff_ctx(fight, managers, |buff_ctx| {
                    buff_ctx.remove_by_uid(spec.target, old.uid);
                });
            }
            let add_layer = if count > 0 {
                count
            } else if cfg_effect_count > 0 {
                cfg_effect_count
            } else if is_layer_stackable {
                1
            } else {
                0
            };
            let use_slave_uid = with_buff_ctx(fight, managers, |buff_ctx| {
                uses_slave_uid(
                    spec.buff_id,
                    &include_types,
                    count,
                    buff_ctx.store,
                    spec.target,
                )
            });
            let buff_add_effect = if use_slave_uid {
                buff_add_slave(spec.target, spec.caster_uid, spec.buff_id, add_layer)
            } else if has_features {
                buff_add_with_count(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    add_layer,
                    cfg_effect_count,
                )
            } else {
                buff_add_with_count(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    add_layer,
                    if is_no_show { 0 } else { cfg_effect_count },
                )
            };
            let buff_uid = buff_add_effect
                .buff
                .as_ref()
                .and_then(|b| b.uid)
                .unwrap_or(0);
            let effect_buff = buff_add_effect.buff.as_ref();
            let initial_stacks = effect_buff.and_then(|b| b.count).unwrap_or(0);
            let initial_layer = effect_buff.and_then(|b| b.layer).unwrap_or(0);
            effects.push(buff_add_effect);
            effects.extend(apply_buff_effects(
                executor,
                fight,
                managers,
                mechanics,
                spec.caster_uid,
                spec.target,
                spec.buff_id,
                spec.has_bloodpool,
            ));
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    spec.target,
                    spec.buff_id,
                    spec.caster_uid,
                    initial_stacks,
                    initial_layer,
                    buff_uid,
                );
            });
            return effects;
        }

        // IncludeType=10 families (non-stack accumulators) re-apply as replace:
        // BUFFDEL + BUFFADD (+ feature effects), matching live semantics.
        let new_count = if count == 0 && has_include_type_2 {
            // includeType=2 buffs (e.g. 30631) should accumulate on re-apply
            // even though they don't use stack layer visuals.
            existing_stacks + add_count
        } else if is_stackable_buff {
            existing_stacks + add_count
        } else {
            add_count
        };

        if is_layer_stackable || is_poison_family {
            let existing_layer = with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx
                    .buffs(spec.target)
                    .iter()
                    .find(|b| b.buff_id == spec.buff_id)
                    .map(|b| b.layer)
                    .unwrap_or(0)
            });
            let add_layer = if count > 0 {
                count
            } else if cfg_effect_count > 0 {
                cfg_effect_count
            } else {
                1
            };
            let base_layer = if existing_layer > 0 {
                existing_layer
            } else {
                1
            };
            let new_layer = base_layer + add_layer;
            let update_count = if existing_stacks > 0 {
                existing_stacks
            } else {
                cfg_effect_count.max(0)
            };
            if is_no_show {
                effects.push(buff_update(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    existing_uid,
                    update_count,
                    new_layer,
                ));
            } else if new_layer > existing_layer.max(1) {
                for layer in (existing_layer.max(1) + 1)..=new_layer {
                    effects.push(buff_update(
                        spec.target,
                        spec.caster_uid,
                        spec.buff_id,
                        existing_uid,
                        update_count,
                        layer,
                    ));
                }
            } else {
                effects.push(buff_update(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    existing_uid,
                    update_count,
                    new_layer,
                ));
            }

            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    spec.target,
                    spec.buff_id,
                    spec.caster_uid,
                    update_count,
                    new_layer,
                    existing_uid,
                );
            });
        } else {
            // Live-style update progression:
            // when stacks increase on an existing buff, emit intermediate BUFFUPDATE
            // frames (e.g. 1 -> 5 emits 2,3,4,5) instead of a single jump.
            if new_count > existing_stacks {
                for stack in (existing_stacks + 1)..=new_count {
                    effects.push(buff_update(
                        spec.target,
                        spec.caster_uid,
                        spec.buff_id,
                        existing_uid,
                        stack,
                        0,
                    ));
                }
            } else {
                effects.push(buff_update(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    existing_uid,
                    new_count,
                    0,
                ));
            }
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    spec.target,
                    spec.buff_id,
                    spec.caster_uid,
                    new_count,
                    0,
                    existing_uid,
                );
            });
        }

        if rerun_post_add_features {
            effects.extend(apply_buff_effects(
                executor,
                fight,
                managers,
                mechanics,
                spec.caster_uid,
                spec.target,
                spec.buff_id,
                spec.has_bloodpool,
            ));
        }
    } else {
        let cfg_type_id = buff_cfg.map(|b| b.type_id).unwrap_or(0);
        let is_stackable_type = cfg
            .skill_bufftype
            .iter()
            .find(|t| t.id == cfg_type_id)
            .map(|t| is_stackable(&t.include_types))
            .unwrap_or(false);
        // Layer should reflect visual stack state only for stackable buff types.
        // Non-stackable buffs should keep layer=0 even if behavior count > 0.
        let add_layer = if count > 0 {
            count
        } else if effect_count > 0 {
            effect_count
        } else if is_stackable_type {
            1
        } else {
            0
        };

        let include_types_for_uid = cfg
            .skill_bufftype
            .iter()
            .find(|t| t.id == cfg_type_id)
            .map(|t| t.include_types.clone())
            .unwrap_or_default();
        let use_slave_uid = with_buff_ctx(fight, managers, |buff_ctx| {
            uses_slave_uid(
                spec.buff_id,
                &include_types_for_uid,
                count,
                buff_ctx.store,
                spec.target,
            )
        });

        let buff_add_effect = if use_slave_uid {
            buff_add_slave(spec.target, spec.caster_uid, spec.buff_id, add_layer)
        } else if has_features {
            buff_add_with_count(
                spec.target,
                spec.caster_uid,
                spec.buff_id,
                add_layer,
                effect_count,
            )
        } else {
            buff_add_with_count(
                spec.target,
                spec.caster_uid,
                spec.buff_id,
                add_layer,
                if is_no_show { 0 } else { effect_count },
            )
        };

        let buff_uid = buff_add_effect
            .buff
            .as_ref()
            .and_then(|b| b.uid)
            .unwrap_or(0);
        let effect_buff = buff_add_effect.buff.as_ref();
        let initial_stacks = effect_buff.and_then(|b| b.count).unwrap_or(0);
        let initial_layer = effect_buff.and_then(|b| b.layer).unwrap_or(0);
        effects.push(buff_add_effect);

        effects.extend(apply_buff_effects(
            executor,
            fight,
            managers,
            mechanics,
            spec.caster_uid,
            spec.target,
            spec.buff_id,
            spec.has_bloodpool,
        ));

        if spec.target > 0 && !is_stackable_type && count > 1 && initial_stacks > 0 {
            for stack in (initial_stacks + 1)..=count {
                effects.push(buff_update(
                    spec.target,
                    spec.caster_uid,
                    spec.buff_id,
                    buff_uid,
                    stack,
                    initial_layer,
                ));
            }
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    spec.target,
                    spec.buff_id,
                    spec.caster_uid,
                    count,
                    initial_layer,
                    buff_uid,
                );
            });
        } else {
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    spec.target,
                    spec.buff_id,
                    spec.caster_uid,
                    initial_stacks,
                    initial_layer,
                    buff_uid,
                );
            });
        }
    }

    effects
}

#[allow(clippy::too_many_arguments)]
pub fn replace_buff2(
    fight: &Fight,
    managers: &mut Managers,
    caster_uid: i64,
    target: i64,
    source_buff_ids: &[i32],
    replacement_buff_id: i32,
    duration: i32,
    count: i32,
) -> Vec<ActEffect> {
    let mut effects = Vec::new();

    let source = with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .iter()
            .find(|b| source_buff_ids.contains(&b.buff_id))
            .cloned()
    });

    if let Some(instance) = source {
        effects.push(buff_del(
            target,
            instance.uid,
            instance.buff_id,
            instance.from_uid,
        ));
        with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx.remove_by_uid(target, instance.uid);
        });
    }

    let existing = with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .iter()
            .find(|b| b.buff_id == replacement_buff_id)
            .cloned()
    });

    let buff_uid = existing
        .map(|b| b.uid)
        .unwrap_or_else(|| next_buff_uid_for_target(target));
    effects.push(buff_update(
        target,
        caster_uid,
        replacement_buff_id,
        buff_uid,
        count,
        0,
    ));
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx.add_with_uid(
            target,
            replacement_buff_id,
            caster_uid,
            duration,
            0,
            buff_uid,
        );
    });
    effects
}

pub fn disperse(fight: &Fight, managers: &mut Managers, target: i64) -> Vec<ActEffect> {
    let cfg = config::configs::get();
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .to_vec()
            .into_iter()
            .filter_map(|instance| {
                let is_good = cfg
                    .skill_buff
                    .iter()
                    .find(|b| b.id == instance.buff_id)
                    .map(|b| {
                        if b.is_good_buff != 0 {
                            true
                        } else {
                            GOOD_BUFF_TYPES.contains(&b.type_id)
                        }
                    })
                    .unwrap_or(false);
                if is_good {
                    let mut e = buff_del(target, instance.uid, instance.buff_id, instance.from_uid);
                    e.config_effect = Some(30003);
                    Some(e)
                } else {
                    None
                }
            })
            .collect()
    })
}

pub fn disperse_force(
    fight: &Fight,
    managers: &mut Managers,
    target: i64,
    buff_id: i32,
) -> Vec<ActEffect> {
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .to_vec()
            .into_iter()
            .filter(|instance| instance.buff_id == buff_id)
            .map(|instance| {
                let mut e = buff_del(target, instance.uid, instance.buff_id, instance.from_uid);
                e.config_effect = Some(30003);
                e
            })
            .collect()
    })
}

pub fn purify(fight: &Fight, managers: &mut Managers, target: i64) -> Vec<ActEffect> {
    let cfg = config::configs::get();
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .to_vec()
            .into_iter()
            .filter_map(|instance| {
                let is_bad = cfg
                    .skill_buff
                    .iter()
                    .find(|b| b.id == instance.buff_id)
                    .map(|b| {
                        if b.is_good_buff != 0 {
                            false
                        } else {
                            BAD_BUFF_TYPES.contains(&b.type_id)
                        }
                    })
                    .unwrap_or(false);
                if is_bad {
                    let mut e = buff_del(target, instance.uid, instance.buff_id, instance.from_uid);
                    e.config_effect = Some(30003);
                    Some(e)
                } else {
                    None
                }
            })
            .collect()
    })
}

pub fn consume_by_type(
    fight: &Fight,
    managers: &mut Managers,
    target: i64,
    type_id: i32,
    skill_id: i32,
    mut count: i32,
) -> Vec<ActEffect> {
    let mut out = Vec::new();
    count = count.max(0);
    if count == 0 {
        return out;
    }

    let has_matching = with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(target)
            .iter()
            .any(|b| b.buff_id == type_id || b.type_id == type_id)
    });
    if !has_matching
        && let Some(seed_layer) =
            super::infer_enter_fight_seed_layer(skill_id, type_id).filter(|v| *v > 0)
    {
        with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx.add(target, type_id, target, 0, seed_layer);
        });
    }

    while count > 0 {
        let candidate = with_buff_ctx(fight, managers, |buff_ctx| {
            buff_ctx
                .buffs(target)
                .iter()
                .filter(|b| b.buff_id == type_id || b.type_id == type_id)
                .min_by_key(|b| b.uid)
                .cloned()
        });
        let Some(buff) = candidate else {
            break;
        };

        if buff.layer > 1 {
            let new_layer = buff.layer - 1;
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    target,
                    buff.buff_id,
                    buff.from_uid,
                    buff.stacks,
                    new_layer,
                    buff.uid,
                );
            });
            out.push(crate::state::battle::utils::buff_update(
                target,
                buff.from_uid,
                buff.buff_id,
                buff.uid,
                buff.stacks,
                new_layer,
            ));
        } else if buff.stacks > 1 {
            let new_count = buff.stacks - 1;
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.add_with_uid(
                    target,
                    buff.buff_id,
                    buff.from_uid,
                    new_count,
                    buff.layer,
                    buff.uid,
                );
            });
            out.push(crate::state::battle::utils::buff_update(
                target,
                buff.from_uid,
                buff.buff_id,
                buff.uid,
                new_count,
                buff.layer,
            ));
        } else {
            with_buff_ctx(fight, managers, |buff_ctx| {
                buff_ctx.remove_by_uid(target, buff.uid);
            });
            out.push(crate::state::battle::utils::buff_del(
                target,
                buff.uid,
                buff.buff_id,
                buff.from_uid,
            ));
        }
        count -= 1;
    }

    out
}

pub fn sum_stacks_by_type(
    fight: &Fight,
    managers: &mut Managers,
    uid: i64,
    buff_type_id: i32,
) -> i32 {
    let cfg = config::configs::get();
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(uid)
            .iter()
            .filter(|b| {
                b.type_id == buff_type_id
                    || cfg
                        .skill_bufftype
                        .iter()
                        .find(|t| t.id == b.type_id)
                        .map(|t| t.r#type == buff_type_id)
                        .unwrap_or(false)
            })
            .map(|b| b.stacks.max(1))
            .sum()
    })
}

pub fn has_any_type(fight: &Fight, managers: &mut Managers, uid: i64, buff_types: &[i32]) -> bool {
    if buff_types.is_empty() {
        return false;
    }
    let cfg = config::configs::get();
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx.buffs(uid).iter().any(|b| {
            cfg.skill_bufftype
                .iter()
                .find(|t| t.id == b.type_id)
                .map(|t| buff_types.contains(&t.r#type))
                .unwrap_or(false)
        })
    })
}

pub fn ban_lost_life_floor_permille(fight: &Fight, managers: &mut Managers, uid: i64) -> i32 {
    with_buff_ctx(fight, managers, |buff_ctx| {
        buff_ctx
            .buffs(uid)
            .iter()
            .find_map(|b| buff_get_ban_lost_life_floor(b.buff_id))
            .unwrap_or(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonettobuf::effect_type_enum::EffectType;
    use std::{path::PathBuf, sync::Once};

    static TEST_CONFIG_INIT: Once = Once::new();

    fn ensure_game_data_initialized() {
        TEST_CONFIG_INIT.call_once(|| {
            if config::configs::try_get().is_some() {
                return;
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let excel_dir = root.join("data").join("excel2json");
            if excel_dir.exists()
                && let Some(path) = excel_dir.to_str()
            {
                let _ = config::configs::init(path);
            }
        });
    }

    #[test]
    fn consume_by_type_emits_ordered_updates() {
        ensure_game_data_initialized();
        let fight = Fight::default();
        let mut managers = Managers::default();
        let uid = 12345_i64;
        let buff_id = 777777_i32;
        let from_uid = uid;

        managers
            .buff_mgr
            .add_with_uid(uid, buff_id, from_uid, 1, 3, 1_000_001);

        let effects = consume_by_type(&fight, &mut managers, uid, buff_id, 0, 2);
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].effect_type, Some(EffectType::Buffupdate as i32));
        assert_eq!(effects[1].effect_type, Some(EffectType::Buffupdate as i32));
        assert_eq!(effects[0].buff.as_ref().and_then(|b| b.layer), Some(2));
        assert_eq!(effects[1].buff.as_ref().and_then(|b| b.layer), Some(1));
    }

    #[test]
    fn purify_and_disperse_pick_expected_buff_kinds() {
        ensure_game_data_initialized();
        let cfg = config::configs::get();
        let Some(&bad_type) = BAD_BUFF_TYPES.first() else {
            return;
        };
        let Some(&good_type) = GOOD_BUFF_TYPES.first() else {
            return;
        };

        let Some(bad_buff) = cfg
            .skill_buff
            .iter()
            .find(|b| b.type_id == bad_type)
            .map(|b| b.id)
        else {
            return;
        };
        let Some(good_buff) = cfg
            .skill_buff
            .iter()
            .find(|b| b.type_id == good_type)
            .map(|b| b.id)
        else {
            return;
        };

        let fight = Fight::default();
        let mut managers = Managers::default();
        let uid = 23456_i64;

        managers
            .buff_mgr
            .add_with_uid(uid, bad_buff, uid, 1, 0, 2_000_001);
        managers
            .buff_mgr
            .add_with_uid(uid, good_buff, uid, 1, 0, 2_000_002);

        let purify_effects = purify(&fight, &mut managers, uid);
        assert!(
            purify_effects
                .iter()
                .all(|e| e.effect_type == Some(EffectType::Buffdel as i32))
        );
        assert!(
            purify_effects
                .iter()
                .any(|e| e.effect_num == Some(bad_buff))
        );
        assert!(
            !purify_effects
                .iter()
                .any(|e| e.effect_num == Some(good_buff))
        );

        let disperse_effects = disperse(&fight, &mut managers, uid);
        assert!(
            disperse_effects
                .iter()
                .all(|e| e.effect_type == Some(EffectType::Buffdel as i32))
        );
        assert!(
            disperse_effects
                .iter()
                .any(|e| e.effect_num == Some(good_buff))
        );
    }
}
