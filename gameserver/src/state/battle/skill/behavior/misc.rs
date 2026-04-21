use anyhow::Result;
use config::configs;
use sonettobuf::{ActEffect, Fight, MagicCircleInfo};

use crate::state::battle::fight_step::ActEffectBuilder;
use crate::state::battle::types::effects::EffectType;
use crate::state::battle::utils::buff_add;

pub fn be_attacked_assassinate() -> Result<Vec<ActEffect>> {
    Ok(vec![])
}

pub fn add_magic_circle(fight: &Fight, caster_uid: i64, circle_id: i32) -> Result<Vec<ActEffect>> {
    let _ = fight;
    let circle = configs::get().magic_circle.get(circle_id).cloned();
    let round = circle.as_ref().map(|circle| circle.round).unwrap_or(0);
    let mut out = Vec::new();
    if let Some(buff_id) = circle
        .as_ref()
        .and_then(|circle| circle.self_buff.trim().parse::<i32>().ok())
        .filter(|id| *id > 0)
    {
        out.push(buff_add(caster_uid, caster_uid, buff_id, 1));
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
pub fn crystal_add_card() -> Result<Vec<ActEffect>> {
    Ok(vec![])
}
