use anyhow::Result;
use sonettobuf::{ActEffect, Fight, FightStep};

use crate::state::battle::{
    fight::defender::Defender, fight_step::FightStepBuilder, types::effects::EffectType,
};

pub fn advance_wave(fight: &mut Fight) -> Result<Vec<FightStep>> {
    let current_wave = fight.cur_wave.unwrap_or(1);
    let new_wave = current_wave + 1;
    let battle_id = fight.battle_id.unwrap_or(0);
    let new_entities = Defender::build_wave_entities(battle_id, new_wave, 2)?;

    let defender = fight
        .defender
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Fight missing defender team"))?;
    defender.entitys = new_entities;
    defender.sub_entitys.clear();

    fight.cur_wave = Some(new_wave);
    fight.is_finish = Some(false);

    Ok(vec![
        FightStepBuilder::effect()
            .with(ActEffect {
                effect_type: Some(EffectType::NewChangeWave as i32),
                effect_num: Some(0),
                fight: Some(fight.clone()),
                ..Default::default()
            })
            .build(),
    ])
}
