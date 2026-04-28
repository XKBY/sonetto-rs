#![allow(dead_code)]

use std::collections::HashSet;

use once_cell::sync::Lazy;

use crate::state::battle::event_queue::SkillEmitKind;

static EQUIPMENT_SKILL_IDS: Lazy<HashSet<i32>> = Lazy::new(|| {
    config::configs::get()
        .equip_skill
        .iter()
        .flat_map(|row| [row.skill, row.skill2])
        .filter(|skill_id| *skill_id > 0)
        .collect()
});

static BATTLE_RULE_EFFECT_IDS: Lazy<HashSet<i32>> = Lazy::new(|| {
    config::configs::get()
        .rule
        .iter()
        .filter_map(|row| row.effect.parse::<i32>().ok())
        .filter(|skill_id| *skill_id > 0)
        .collect()
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEmitContext {
    PlayerCardPlay,
    PassiveWalker,
    EventTriggerWalker,
    Other,
}

pub fn classify_skill_emit(skill_id: i32, caller_context: SkillEmitContext) -> SkillEmitKind {
    if is_equipment_skill(skill_id) {
        return SkillEmitKind::EquipmentEmbedded;
    }

    match caller_context {
        SkillEmitContext::PlayerCardPlay => SkillEmitKind::PlayerInitiated,
        SkillEmitContext::PassiveWalker => SkillEmitKind::AutomaticPhase,
        SkillEmitContext::EventTriggerWalker => SkillEmitKind::EventTriggered,
        SkillEmitContext::Other => {
            if is_battle_rule_derived(skill_id) {
                return SkillEmitKind::AutomaticPhase;
            }
            SkillEmitKind::PlayerInitiated
        }
    }
}

fn is_equipment_skill(skill_id: i32) -> bool {
    EQUIPMENT_SKILL_IDS.contains(&skill_id)
}

fn is_battle_rule_derived(skill_id: i32) -> bool {
    BATTLE_RULE_EFFECT_IDS.contains(&skill_id)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Once};

    use super::{SkillEmitContext, classify_skill_emit};
    use crate::state::battle::event_queue::SkillEmitKind;

    static TEST_CONFIG_INIT: Once = Once::new();

    fn ensure_game_data_initialized() {
        TEST_CONFIG_INIT.call_once(|| {
            if config::configs::try_get().is_some() {
                return;
            }

            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let excel_dir = root.join("data").join("excel2json");
            if excel_dir.exists()
                && let Some(path) = excel_dir.to_str()
            {
                let _ = config::configs::init(path);
            }
        });
    }

    #[test]
    fn classifies_recoleta_equipment_skill_as_equipment_embedded() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(434415, SkillEmitContext::Other),
            SkillEmitKind::EquipmentEmbedded,
        );
    }

    #[test]
    fn classifies_rubuska_equipment_skill_as_equipment_embedded() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(435611, SkillEmitContext::Other),
            SkillEmitKind::EquipmentEmbedded,
        );
    }

    #[test]
    fn classifies_battle_rule_skill_as_automatic_phase() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(530000151, SkillEmitContext::Other),
            SkillEmitKind::AutomaticPhase,
        );
    }

    #[test]
    fn classifies_player_card_play_as_player_initiated() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(30090111, SkillEmitContext::PlayerCardPlay),
            SkillEmitKind::PlayerInitiated,
        );
    }

    #[test]
    fn classifies_passive_walker_as_automatic_phase() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(31040141, SkillEmitContext::PassiveWalker),
            SkillEmitKind::AutomaticPhase,
        );
    }

    #[test]
    fn classifies_trigger_reactive_walker_as_event_triggered() {
        ensure_game_data_initialized();
        assert_eq!(
            classify_skill_emit(31260181, SkillEmitContext::EventTriggerWalker),
            SkillEmitKind::EventTriggered,
        );
    }
}
