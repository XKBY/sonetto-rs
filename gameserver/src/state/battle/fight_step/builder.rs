use sonettobuf::{
    ActEffect, BuffActInfo, FightHurtInfo, FightStep, MagicCircleInfo,
    effect_type_enum::EffectType, fight_step,
};

pub struct ActEffectBuilder {
    effect: ActEffect,
}

#[allow(dead_code)]
impl ActEffectBuilder {
    pub fn new(effect_type: i32, target_id: i64) -> Self {
        Self {
            effect: ActEffect {
                effect_type: Some(effect_type),
                target_id: Some(target_id),
                ..Default::default()
            },
        }
    }

    pub fn ex_point_change(target: i64, delta: i32) -> ActEffect {
        Self::new(EffectType::Expointchange as i32, target)
            .effect_num(delta)
            .build()
    }

    pub fn bloodpool_value_change(target: i64, team: i32, delta: i32) -> ActEffect {
        Self::new(EffectType::Bloodpoolvaluechange as i32, target)
            .effect_num(team)
            .effect_num1(delta)
            .build()
    }

    pub fn bloodpool_max_change(team: i32, amount: i32) -> ActEffect {
        Self::new(EffectType::Bloodpoolmaxchange as i32, 0)
            .effect_num(team)
            .effect_num1(amount)
            .build()
    }

    pub fn effect_num(mut self, value: i32) -> Self {
        self.effect.effect_num = Some(value);
        self
    }

    pub fn effect_num1(mut self, value: i32) -> Self {
        self.effect.effect_num1 = Some(value);
        self
    }

    pub fn config_effect(mut self, value: i32) -> Self {
        self.effect.config_effect = Some(value);
        self
    }

    pub fn buff_act_id(mut self, value: i32) -> Self {
        self.effect.buff_act_id = Some(value);
        self
    }

    pub fn reserve_id(mut self, value: i64) -> Self {
        self.effect.reserve_id = Some(value);
        self
    }

    pub fn reserve_str(mut self, value: impl Into<String>) -> Self {
        self.effect.reserve_str = Some(value.into());
        self
    }

    pub fn team_type(mut self, value: i32) -> Self {
        self.effect.team_type = Some(value);
        self
    }

    pub fn hurt_info(mut self, value: FightHurtInfo) -> Self {
        self.effect.hurt_info = Some(value);
        self
    }

    pub fn buff_act_info(mut self, value: BuffActInfo) -> Self {
        self.effect.buff_act_info = Some(value);
        self
    }

    pub fn magic_circle(mut self, value: MagicCircleInfo) -> Self {
        self.effect.magic_circle = Some(value);
        self
    }

    pub fn fight_step(mut self, value: FightStep) -> Self {
        self.effect.fight_step = Some(value);
        self
    }

    pub fn build(self) -> ActEffect {
        self.effect
    }
}

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
        let step = effect_container_step(from_uid, to_uid, act_id, effects);
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
        self.effects.push(
            ActEffectBuilder::new(EffectType::Bloodpoolmaxchange as i32, 0)
                .team_type(team)
                .effect_num(1)
                .effect_num1(max)
                .build(),
        );
        self.effects.push(
            ActEffectBuilder::new(EffectType::Bloodpoolvaluechange as i32, display_uid)
                .team_type(team)
                .effect_num(cur)
                .effect_num1(1)
                .build(),
        );
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

pub fn effect_container_step(
    from_uid: i64,
    to_uid: i64,
    act_id: i32,
    effects: Vec<ActEffect>,
) -> FightStep {
    FightStep {
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
    }
}

pub fn wrap_step(step: FightStep) -> ActEffect {
    ActEffectBuilder::new(EffectType::Fightstep as i32, 0)
        .effect_num(0)
        .fight_step(step)
        .build()
}
