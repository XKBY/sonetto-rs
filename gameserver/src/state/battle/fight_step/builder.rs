use sonettobuf::{ActEffect, FightStep, effect_type_enum::EffectType, fight_step};

pub struct FightStepBuilder {
    act_type: fight_step::ActType,
    from_id: i64,
    to_id: i64,
    act_id: i32,
    effects: Vec<ActEffect>,
}

#[allow(dead_code)]
impl FightStepBuilder {
    pub fn effect() -> Self {
        Self {
            act_type: fight_step::ActType::Effect,
            from_id: 0,
            to_id: 0,
            act_id: 0,
            effects: vec![],
        }
    }

    pub fn effect_from(from_id: i64) -> Self {
        Self {
            act_type: fight_step::ActType::Effect,
            from_id,
            to_id: 0,
            act_id: 0,
            effects: vec![],
        }
    }

    pub fn skill(from_id: i64, to_id: i64, skill_id: i32) -> Self {
        Self {
            act_type: fight_step::ActType::Skill,
            from_id,
            to_id,
            act_id: skill_id,
            effects: vec![],
        }
    }

    pub fn with(mut self, effect: ActEffect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_many(mut self, effects: Vec<ActEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn with_nested(mut self, step: FightStep) -> Self {
        self.effects.push(ActEffect {
            effect_type: Some(EffectType::Fightstep as i32),
            fight_step: Some(step),
            target_id: Some(0),
            ..Default::default()
        });
        self
    }

    pub fn with_skill_container(
        mut self,
        from_uid: i64,
        skill_id: i32,
        effects: Vec<ActEffect>,
    ) -> Self {
        let step = FightStepBuilder::skill(from_uid, from_uid, skill_id)
            .with_many(effects)
            .build();
        self.effects.push(wrap_step(step));
        self
    }

    pub fn with_effect_container(
        mut self,
        from_uid: i64,
        to_uid: i64,
        act_id: i32,
        effects: Vec<ActEffect>,
    ) -> Self {
        let step = FightStep {
            act_type: Some(fight_step::ActType::Effect.into()),
            from_id: Some(from_uid),
            to_id: Some(to_uid),
            act_id: Some(act_id),
            act_effect: effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        };
        self.effects.push(wrap_step(step));
        self
    }

    pub fn with_bloodtithe_sync(mut self, team: i32, display_uid: i64, cur: i32, max: i32) -> Self {
        self.effects.push(ActEffect {
            effect_type: Some(EffectType::Bloodpoolmaxcreate as i32),
            target_id: Some(0),
            team_type: Some(team),
            effect_num: Some(1),
            ..Default::default()
        });
        self.effects.push(ActEffect {
            effect_type: Some(EffectType::Bloodpoolmaxchange as i32),
            target_id: Some(0),
            team_type: Some(team),
            effect_num: Some(1),
            effect_num1: Some(max),
            ..Default::default()
        });
        self.effects.push(ActEffect {
            effect_type: Some(EffectType::Bloodpoolvaluechange as i32),
            target_id: Some(display_uid),
            team_type: Some(team),
            effect_num: Some(cur),
            effect_num1: Some(1),
            ..Default::default()
        });
        self
    }

    pub fn build(self) -> FightStep {
        FightStep {
            act_type: Some(self.act_type.into()),
            from_id: Some(self.from_id),
            to_id: Some(self.to_id),
            act_id: Some(self.act_id),
            act_effect: self.effects,
            card_index: Some(0),
            support_hero_id: Some(0),
            fake_timeline: Some(false),
            real_skill_type: Some(0),
            real_skin_id: Some(0),
        }
    }

    pub fn wrap(self) -> ActEffect {
        wrap_step(self.build())
    }
}

pub fn wrap_step(step: FightStep) -> ActEffect {
    ActEffect {
        effect_type: Some(EffectType::Fightstep as i32),
        target_id: Some(0),
        effect_num: Some(0),
        fight_step: Some(step),
        ..Default::default()
    }
}
