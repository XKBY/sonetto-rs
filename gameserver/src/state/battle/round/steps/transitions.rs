use sonettobuf::{ActEffect, FightStep, fight_step};

fn effect(effect_type: i32, effect_num: i32, team_type: Option<i32>) -> ActEffect {
    ActEffect {
        effect_type: Some(effect_type),
        target_id: Some(0),
        effect_num: Some(effect_num),
        team_type,
        ..Default::default()
    }
}

fn effect_step(effects: Vec<ActEffect>) -> FightStep {
    FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        from_id: Some(0),
        to_id: Some(0),
        act_id: Some(0),
        act_effect: effects,
        card_index: Some(0),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}

/// Live-style transition block right before enemy cards start:
///   ROUNDEND(61), SMALLROUNDEND(211), DEALCARD2(60), CARDDECKNUM(310)
pub fn build_pre_enemy_transition_steps(deck_num: i32) -> Vec<FightStep> {
    vec![
        effect_step(vec![
            effect(61, 0, None),
            effect(211, 0, None),
            effect(60, 0, None),
        ]),
        effect_step(vec![effect(310, deck_num, Some(1))]),
    ]
}

/// Live-style turn close-out block after enemy actions when battle is still ongoing:
///   SMALLROUNDEND(211), CLEARUNIVERSALCARD(96), CHANGEROUND(212), CARDDECKNUM(310)
#[allow(dead_code)]
pub fn build_post_enemy_transition_steps(deck_num: i32) -> Vec<FightStep> {
    vec![
        effect_step(vec![effect(211, 0, None)]),
        effect_step(vec![effect(96, 0, None)]),
        effect_step(vec![effect(212, 0, None)]),
        effect_step(vec![effect(310, deck_num, Some(1))]),
    ]
}
