use sonettobuf::ActEffect;

/// What a buff_action handler can produce.
/// Most handlers only need `effects`. The others are opt-in.
#[derive(Default)]
pub struct ActionResult {
    pub effects: Vec<ActEffect>,
    pub side_effects: Vec<ActEffect>,
    pub buff_dels: Vec<(i64, i32)>,
    pub monitor_triggers: Vec<(i64, i32)>,
}

impl ActionResult {
    pub fn none(target: i64) -> Self {
        Self {
            effects: vec![crate::state::battle::fight_step::ActEffectBuilder::effect_none(target)],
            ..Default::default()
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn effects(effects: Vec<ActEffect>) -> Self {
        Self {
            effects,
            ..Default::default()
        }
    }

    pub fn single(effect: ActEffect) -> Self {
        Self {
            effects: vec![effect],
            ..Default::default()
        }
    }
}
