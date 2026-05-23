use sonettobuf::Fight;

use crate::state::battle::skill::cache::resolve_skill_effect_id;

pub(crate) fn collect_battle_rule_skills(fight: &Fight) -> Vec<i32> {
    collect_rules(fight)
        .into_iter()
        .map(|(_, effect_id)| effect_id)
        .collect()
}

pub(crate) fn collect_rules(fight: &Fight) -> Vec<(i32, i32)> {
    let cfg = config::configs::get();
    let battle_id = fight.battle_id.unwrap_or(0);
    let Some(battle) = cfg.battle.iter().find(|b| b.id == battle_id) else {
        tracing::info!("collect_rules: no battle found for battle_id={}", battle_id);
        return vec![];
    };

    let mut out = Vec::new();
    for rule_str in [&battle.addition_rule, &battle.hidden_rule] {
        if rule_str.is_empty() {
            continue;
        }
        tracing::info!(
            "collect_rules: battle_id={} rule_str={}",
            battle_id,
            rule_str
        );
        for entry in rule_str.split('|') {
            let mut parts = entry.split('#');
            let Some(prefix) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            if !(1..=3).contains(&prefix) {
                continue;
            }
            let Some(skill_id) = parts.next().and_then(|v| v.parse::<i32>().ok()) else {
                continue;
            };
            let effect_id = resolve_skill_effect_id(skill_id);
            out.push((prefix, effect_id));
        }
    }
    out
}
