use sonettobuf::{ActEffect, FightStep, fight_step};

use crate::state::battle::context::FightContext;

pub fn build_temp_card_step(ctx: &mut FightContext<'_>, uids: &[i64]) -> Option<FightStep> {
    let cfg = config::configs::get();
    let mut effects: Vec<ActEffect> = Vec::new();

    for &uid in uids {
        for instance in ctx.managers.buff_mgr.get(uid).to_vec() {
            let Some(buff_cfg) = cfg.skill_buff.iter().find(|b| b.id == instance.buff_id) else {
                continue;
            };
            for entry in buff_cfg.features.split('|') {
                let parts: Vec<&str> = entry.split('#').collect();
                let act_id: i32 = parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                let act_type = cfg
                    .buff_act
                    .iter()
                    .find(|a| a.id == act_id)
                    .map(|a| a.r#type.as_str())
                    .unwrap_or("");
                if act_type != "AddSpTempCard" {
                    continue;
                }

                let ex_skill_id: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let model_id = ctx
                    .fight
                    .attacker
                    .as_ref()
                    .and_then(|a| a.entitys.iter().find(|e| e.uid == Some(uid)))
                    .and_then(|e| e.model_id)
                    .unwrap_or(0);
                let ex_max = ctx.managers.ex_point_mgr.get_ex_max(uid);

                let inner = FightStep {
                    act_type: Some(fight_step::ActType::Effect.into()),
                    from_id: Some(uid),
                    to_id: Some(uid),
                    act_id: Some(instance.buff_id),
                    act_effect: vec![
                        ActEffect {
                            effect_type: Some(78),
                            target_id: Some(uid),
                            effect_num: Some(ex_skill_id),
                            reserve_id: Some(model_id as i64),
                            team_type: Some(1),
                            ..Default::default()
                        },
                        ActEffect {
                            effect_type: Some(141),
                            target_id: Some(uid),
                            reserve_str: Some(ex_max.to_string()),
                            team_type: Some(1),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                };
                effects.push(ActEffect {
                    effect_type: Some(sonettobuf::effect_type_enum::EffectType::Fightstep as i32),
                    target_id: Some(0),
                    effect_num: Some(0),
                    fight_step: Some(inner),
                    ..Default::default()
                });
            }
        }
    }

    if effects.is_empty() {
        None
    } else {
        Some(
            crate::state::battle::fight_step::FightStepBuilder::effect()
                .with_many(effects)
                .build(),
        )
    }
}

pub fn build_temp_card_cleanup_step(ctx: &mut FightContext<'_>, uids: &[i64]) -> Option<FightStep> {
    let cfg = config::configs::get();
    let mut effects: Vec<ActEffect> = Vec::new();

    for &uid in uids {
        let to_remove: Vec<_> = ctx
            .managers
            .buff_mgr
            .get(uid)
            .iter()
            .filter(|instance| {
                cfg.skill_buff
                    .iter()
                    .find(|b| b.id == instance.buff_id)
                    .map(|b| {
                        b.features.split('|').any(|entry| {
                            let parts: Vec<&str> = entry.split('#').collect();
                            let act_id: i32 =
                                parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                            cfg.buff_act
                                .iter()
                                .find(|a| a.id == act_id)
                                .map(|a| a.r#type == "AddSpTempCard")
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
            .map(|i| (i.uid, i.buff_id, i.from_uid))
            .collect();

        for (buff_uid, buff_id, from_uid) in to_remove {
            effects.push(crate::state::battle::fight_step::ActEffectBuilder::buff_del(uid, buff_uid, buff_id, from_uid));
            // intentionally NOT removing from buff_mgr - buff persists for ReplaceBuff2
        }
    }

    if effects.is_empty() {
        None
    } else {
        Some(
            crate::state::battle::fight_step::FightStepBuilder::effect()
                .with_many(effects)
                .build(),
        )
    }
}


