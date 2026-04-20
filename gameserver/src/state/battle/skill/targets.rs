use sonettobuf::{Fight, FightEntityInfo};

pub fn get_entity(fight: &Fight, uid: i64) -> Option<&FightEntityInfo> {
    if let Some(a) = &fight.attacker
        && let Some(e) = a
            .entitys
            .iter()
            .chain(a.sub_entitys.iter())
            .find(|e| e.uid == Some(uid))
    {
        return Some(e);
    }

    if let Some(d) = &fight.defender
        && let Some(e) = d
            .entitys
            .iter()
            .chain(d.sub_entitys.iter())
            .find(|e| e.uid == Some(uid))
    {
        return Some(e);
    }
    None
}

pub fn get_team_type(fight: &Fight, uid: i64) -> Option<i32> {
    get_entity(fight, uid).and_then(|e| e.team_type)
}

pub fn collect_team(fight: &Fight, caster_team: Option<i32>, same_side: bool) -> Vec<i64> {
    let mut uids = vec![];
    let check = |e_team: Option<i32>| {
        if same_side {
            e_team == caster_team
        } else {
            e_team != caster_team
        }
    };
    let is_active =
        |e: &FightEntityInfo| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0;
    if let Some(a) = &fight.attacker {
        for e in a.entitys.iter().chain(a.sub_entitys.iter()) {
            if check(e.team_type)
                && is_active(e)
                && let Some(uid) = e.uid
            {
                uids.push(uid);
            }
        }
    }
    if let Some(d) = &fight.defender {
        for e in d.entitys.iter().chain(d.sub_entitys.iter()) {
            if check(e.team_type)
                && is_active(e)
                && let Some(uid) = e.uid
            {
                uids.push(uid);
            }
        }
    }
    uids
}

pub fn get_ally_uids(fight: &Fight, caster_uid: i64) -> Vec<i64> {
    let in_attacker = fight
        .attacker
        .as_ref()
        .map(|a| a.entitys.iter().any(|e| e.uid == Some(caster_uid)))
        .unwrap_or(false);
    let side = if in_attacker {
        fight.attacker.as_ref()
    } else {
        fight.defender.as_ref()
    };
    side.map(|a| {
        a.entitys
            .iter()
            .filter(|e| e.position.unwrap_or(-1) > 0 && e.current_hp.unwrap_or(0) > 0)
            .filter_map(|e| e.uid)
            .collect()
    })
    .unwrap_or_default()
}

pub fn resolve_targets(
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    behavior_target: i32,
    condition_target: i32,
    logic_target: i32,
) -> Vec<i64> {
    let caster_team = get_team_type(fight, caster_uid);
    let mut current_target = behavior_target;
    let mut current_condition_target = condition_target;

    loop {
        match current_target {
            0 => {
                // 0 means "defer to logic target". If logic target is also unresolved,
                // avoid redirect recursion and keep the runtime-selected target.
                if logic_target == 0 {
                    return if target_uid != 0 {
                        vec![target_uid]
                    } else {
                        vec![caster_uid]
                    };
                }
                current_target = logic_target;
            }

            103 => return vec![caster_uid],

            101 | 104 | 105 => return collect_team(fight, caster_team, true),

            102 => {
                return collect_team(fight, caster_team, true)
                    .into_iter()
                    .filter(|&uid| uid != caster_uid)
                    .collect();
            }

            1 | 106 | 107 | 108 | 204 | 205 | 207 | 303 => return vec![target_uid],

            109 => {
                return collect_team(fight, caster_team, true)
                    .into_iter()
                    .min_by(|&a, &b| {
                        let hp_pct = |uid: i64| {
                            get_entity(fight, uid)
                                .map(|e| {
                                    let cur = e.current_hp.unwrap_or(0) as f32;
                                    let max = e.attr.as_ref().and_then(|a| a.hp).unwrap_or(1) as f32;
                                    (cur / max * 10000.0) as i32
                                })
                                .unwrap_or(i32::MAX)
                        };
                        hp_pct(a).cmp(&hp_pct(b))
                    })
                    .map(|uid| vec![uid])
                    .unwrap_or_default();
            }

            128 => {
                // adjacent ally in front (position - 1)
                let caster_pos = get_entity(fight, caster_uid)
                    .and_then(|e| e.position)
                    .unwrap_or(0);
                let target_pos = caster_pos - 1;
                if target_pos <= 0 {
                    return vec![];
                }
                let side = if fight
                    .attacker
                    .as_ref()
                    .map(|a| a.entitys.iter().any(|e| e.uid == Some(caster_uid)))
                    .unwrap_or(false)
                {
                    fight.attacker.as_ref()
                } else {
                    fight.defender.as_ref()
                };
                return side
                    .and_then(|s| {
                        s.entitys
                            .iter()
                            .chain(s.sub_entitys.iter())
                            .find(|e| e.position == Some(target_pos) && e.current_hp.unwrap_or(0) > 0)
                            .and_then(|e| e.uid)
                    })
                    .map(|uid| vec![uid])
                    .unwrap_or_default();
            }
            201 => {
                let mut enemies = collect_team(fight, caster_team, false);
                enemies.sort_by_key(|&uid| {
                    get_entity(fight, uid)
                        .and_then(|e| e.position)
                        .unwrap_or(99)
                });
                if enemies.is_empty() {
                    return vec![];
                }

                let mut out: Vec<i64> = Vec::new();
                if enemies.contains(&target_uid) {
                    out.push(target_uid);
                    if enemies.len() > 1 {
                        let idx = enemies.iter().position(|uid| *uid == target_uid).unwrap_or(0);
                        let extra = enemies[(idx + 1) % enemies.len()];
                        if extra != target_uid {
                            out.push(extra);
                        }
                    }
                } else {
                    let mut by_hp = enemies.clone();
                    by_hp.sort_by(|&a, &b| {
                        let hp = |uid: i64| {
                            get_entity(fight, uid)
                                .and_then(|e| e.current_hp)
                                .unwrap_or(0)
                        };
                        hp(b)
                            .cmp(&hp(a))
                            .then_with(|| {
                                let pos = |uid: i64| {
                                    get_entity(fight, uid)
                                        .and_then(|e| e.position)
                                        .unwrap_or(99)
                                };
                                pos(a).cmp(&pos(b))
                            })
                    });
                    out.push(by_hp[0]);
                    if by_hp.len() > 1 {
                        out.push(by_hp[1]);
                    }
                }
                return out;
            }
            206 => return vec![target_uid],
            112 => {
                return collect_team(fight, caster_team, true)
                    .into_iter()
                    .max_by(|&a, &b| {
                        let a_ex = get_entity(fight, a).and_then(|e| e.ex_point).unwrap_or(0);
                        let b_ex = get_entity(fight, b).and_then(|e| e.ex_point).unwrap_or(0);
                        a_ex.cmp(&b_ex)
                    })
                    .map(|uid| vec![uid])
                    .unwrap_or_else(|| vec![caster_uid]);
            }

            202 => return collect_team(fight, caster_team, false),

            208 => {
                // Target enemy with the most current HP
                return collect_team(fight, caster_team, false)
                    .into_iter()
                    .max_by(|&a, &b| {
                        let hp = |uid: i64| {
                            get_entity(fight, uid)
                                .and_then(|e| e.current_hp)
                                .unwrap_or(0)
                        };
                        hp(a).cmp(&hp(b))
                    })
                    .map(|uid| vec![uid])
                    .unwrap_or_default();
            }

            301 | 302 => return collect_team(fight, caster_team, false),
            // Main target of a multi-target action.
            233 => return vec![target_uid],

            226..=235 => {
                let pos = current_target - 225;
                let mut enemies = collect_team(fight, caster_team, false);
                enemies.sort_by_key(|&uid| {
                    get_entity(fight, uid)
                        .and_then(|e| e.position)
                        .unwrap_or(99)
                });
                return enemies
                    .into_iter()
                    .filter(|&uid| {
                        get_entity(fight, uid)
                            .and_then(|e| e.position)
                            .map(|p| p == pos)
                            .unwrap_or(false)
                    })
                    .collect();
            }

            999 => {
                // 999 usually follows the event/runtime-selected target lane.
                // When condition target is empty, keep config-driven fan-out semantics
                // via logic target (e.g. active skills that broadcast to enemy side).
                if current_condition_target == 0 {
                    let effective = if logic_target != 0 {
                        logic_target
                    } else if target_uid != 0 {
                        target_uid as i32
                    } else {
                        103
                    };
                    if effective == 999 {
                        return if target_uid != 0 {
                            vec![target_uid]
                        } else {
                            vec![caster_uid]
                        };
                    }
                    current_target = effective;
                } else if target_uid != 0 {
                    return vec![target_uid];
                } else {
                    current_target = current_condition_target;
                    current_condition_target = 0;
                }
            }

            _ => {
                tracing::warn!("resolve_targets: unhandled target type {}", current_target);
                return vec![caster_uid];
            }
        }
    }
}

pub fn resolve_behavior_targets(
    fight: &Fight,
    caster_uid: i64,
    target_uid: i64,
    behavior_target: i32,
    condition_target: i32,
    logic_target: i32,
    add_buff_fanout_999: bool,
) -> Vec<i64> {
    if add_buff_fanout_999
        && behavior_target == 999
        && condition_target == 101
        && logic_target == 1
    {
        return collect_team(fight, get_team_type(fight, caster_uid), true);
    }

    resolve_targets(
        fight,
        caster_uid,
        target_uid,
        behavior_target,
        condition_target,
        logic_target,
    )
}
