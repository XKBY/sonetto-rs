use rand::rngs::StdRng;
use sonettobuf::Fight;
use crate::state::battle::{
    event::Event,
    fight_step::ActEffectBuilder,
    manager::fight_data_mgr::Managers,
    mechanics::{Mechanics, dot::parse_dot_features},
    skill::{SkillExecutor, get_entity},
    utils::apply_real_hurt_fix,
};
use crate::state::battle::effect::behaviour_type::BehaviourType;

const CRIT_PERMILLE: i32 = 1390;
const SETTLE_CONFIG_EFFECT: i32 = 60073;

pub fn execute(
    fight: &Fight, managers: &mut Managers, _mechanics: &mut Mechanics,
    _executor: &mut SkillExecutor, _rng: &mut StdRng,
    _targets: Vec<i64>, entity_uid: i64, _raw: &str, _count: i32, _beh_type: BehaviourType,
) -> Vec<Event> {
    let carrier = entity_uid;
    if carrier == 0 {
        return vec![];
    }

    let buffs = managers.buff_mgr.get(carrier).to_vec();
    if buffs.is_empty() {
        return vec![];
    }

    let mut total_damage: i32 = 0;
    for instance in &buffs {
        let Some((_marker_et, permille)) = parse_dot_features(instance.buff_id) else {
            continue;
        };
        let Some(poisoner) = get_entity(fight, instance.from_uid) else {
            continue;
        };
        let poisoner_atk = poisoner.attr.as_ref().and_then(|a| a.attack).unwrap_or(0);
        if poisoner_atk <= 0 {
            continue;
        }
        let base = apply_real_hurt_fix(
            &managers.buff_mgr,
            carrier,
            poisoner_atk * permille / 1000,
        );
        if base <= 0 {
            continue;
        }
        let stacks = instance.layer.max(1);
        let crit_dmg = base.saturating_mul(CRIT_PERMILLE) / 1000;
        total_damage = total_damage.saturating_add(crit_dmg.saturating_mul(stacks));
    }

    if total_damage <= 0 {
        return vec![];
    }

    let mut effects = vec![Event::SerializedActEffect {
        effect: ActEffectBuilder::origin_crit(carrier, total_damage, Some(SETTLE_CONFIG_EFFECT)),
    }];

    if let Some(victim) = get_entity(fight, carrier) {
        let hp = victim.current_hp.unwrap_or(0);
        let shield = victim.shield_value.unwrap_or(0);
        let after_shield = total_damage.saturating_sub(shield);
        if after_shield > 0 && hp - after_shield <= 0 {
            effects.push(Event::SerializedActEffect { effect: ActEffectBuilder::dead(carrier) });
            effects.push(Event::SerializedActEffect { effect: ActEffectBuilder::remove_entity_cards(carrier, Some(1)) });
        }
    }

    effects
}
