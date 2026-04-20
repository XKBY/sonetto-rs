use serde_json::{Map, Value};
use sonettobuf::card_info::{CardStatus, CardType};
use sonettobuf::fight::FightActType;
use sonettobuf::fight_hurt_info::DamageFromType;

fn reorder_object_with_keys(obj: &mut Map<String, Value>, preferred: &[&str]) {
    let mut existing: Vec<(String, Value)> = std::mem::take(obj).into_iter().collect();

    for &k in preferred {
        if let Some(idx) = existing.iter().position(|(ek, _)| ek == k) {
            let (key, value) = existing.remove(idx);
            obj.insert(key, value);
        }
    }

    for (k, v) in existing {
        obj.insert(k, v);
    }
}

const MAX_FORMAT_DEPTH: usize = 64;

pub fn apply_live_format(value: &mut Value) {
    normalize_direct_use_bigskill_sequences(value);
    apply_live_format_inner(value, 0);
}

fn normalize_direct_use_bigskill_sequences(root: &mut Value) {
    fn skill_need_ex_point(skill_id: i32) -> Option<i32> {
        let cfg = config::configs::get();
        let resolved_id = if cfg.skill_effect.iter().any(|s| s.id == skill_id) {
            skill_id
        } else {
            cfg.skill
                .iter()
                .find(|s| s.id == skill_id)
                .map(|s| s.skill_effect)
                .unwrap_or(skill_id)
        };
        cfg.skill_effect
            .iter()
            .find(|s| s.id == resolved_id)
            .map(|s| s.need_ex_point.max(0))
    }

    fn as_i32(v: Option<&Value>) -> Option<i32> {
        v.and_then(Value::as_i64).map(|n| n as i32)
    }

    fn buff_effect_count(buff_id: i32) -> Option<i32> {
        let cfg = config::configs::get();
        cfg.skill_buff
            .iter()
            .find(|b| b.id == buff_id)
            .map(|b| b.effect_count.max(0))
    }

    fn normalize_step(step_obj: &mut Map<String, Value>) {
        let Some(Value::Array(act_effects)) = step_obj.get_mut("actEffect") else {
            return;
        };
        if act_effects.len() < 5 {
            return;
        }

        for i in 0..=act_effects.len().saturating_sub(5) {
            let e0 = act_effects.get(i);
            let e1 = act_effects.get(i + 1);
            let e2 = act_effects.get(i + 2);
            let e3 = act_effects.get(i + 3);
            let e4 = act_effects.get(i + 4);

            let pattern_matches = as_i32(
                e0.and_then(Value::as_object)
                    .and_then(|o| o.get("effectType")),
            ) == Some(162)
                && as_i32(
                    e1.and_then(Value::as_object)
                        .and_then(|o| o.get("effectType")),
                ) == Some(111)
                && as_i32(
                    e2.and_then(Value::as_object)
                        .and_then(|o| o.get("effectType")),
                ) == Some(327)
                && as_i32(
                    e3.and_then(Value::as_object)
                        .and_then(|o| o.get("effectType")),
                ) == Some(162)
                && as_i32(
                    e4.and_then(Value::as_object)
                        .and_then(|o| o.get("effectType")),
                ) == Some(111);

            if !pattern_matches {
                continue;
            }

            let ex_skill_id = e3
                .and_then(Value::as_object)
                .and_then(|o| o.get("fightStep"))
                .and_then(Value::as_object)
                .and_then(|s| as_i32(s.get("actId")));
            let Some(ex_skill_id) = ex_skill_id else {
                continue;
            };

            let prep_buff_id = e0
                .and_then(Value::as_object)
                .and_then(|o| o.get("fightStep"))
                .and_then(Value::as_object)
                .and_then(|s| s.get("actEffect"))
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_object)
                .and_then(|ae| ae.get("buff"))
                .and_then(Value::as_object)
                .and_then(|b| as_i32(b.get("buffId")));

            let Some(need_ex) = skill_need_ex_point(ex_skill_id)
                .or_else(|| prep_buff_id.and_then(buff_effect_count))
                .filter(|v| *v > 0)
            else {
                continue;
            };

            if let Some(obj) = act_effects.get_mut(i + 1).and_then(Value::as_object_mut) {
                obj.insert("effectNum".to_string(), Value::from(-need_ex));
            }
            if let Some(obj) = act_effects.get_mut(i + 4).and_then(Value::as_object_mut) {
                obj.insert("effectNum".to_string(), Value::from(need_ex));
            }

            if let Some(prep_step) = act_effects
                .get_mut(i)
                .and_then(Value::as_object_mut)
                .and_then(|o| o.get_mut("fightStep"))
                .and_then(Value::as_object_mut)
                && let Some(Value::Array(prep_effects)) = prep_step.get_mut("actEffect")
            {
                for prep in prep_effects.iter_mut() {
                    let Some(prep_obj) = prep.as_object_mut() else {
                        continue;
                    };
                    if as_i32(prep_obj.get("effectType")) != Some(5) {
                        continue;
                    }
                    if let Some(buff_obj) = prep_obj.get_mut("buff").and_then(Value::as_object_mut)
                    {
                        buff_obj.insert("layer".to_string(), Value::from(need_ex));
                    }
                }
            }
        }
    }

    fn visit(v: &mut Value) {
        match v {
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    visit(item);
                }
            }
            Value::Object(obj) => {
                normalize_step(obj);
                for item in obj.values_mut() {
                    visit(item);
                }
            }
            _ => {}
        }
    }

    visit(root);
}

fn apply_live_format_inner(value: &mut Value, depth: usize) {
    if depth > MAX_FORMAT_DEPTH {
        return;
    }

    match value {
        Value::Array(arr) => {
            for v in arr {
                apply_live_format_inner(v, depth + 1);
            }
        }
        Value::Object(obj) => {
            for v in obj.values_mut() {
                apply_live_format_inner(v, depth + 1);
            }

            if let Some(v) = obj.get_mut("fightActType")
                && let Some(n) = v.as_i64()
                && let Ok(kind) = <FightActType as TryFrom<i32>>::try_from(n as i32)
            {
                *v = Value::from(kind.as_str_name());
            }

            let keys: std::collections::HashSet<_> = obj.keys().map(String::as_str).collect();

            let start_dungeon_reply_order = ["fight", "round"];
            let fight_order = [
                "param",
                "customData",
                "progressList",
                "attacker",
                "defender",
                "curRound",
                "maxRound",
                "isFinish",
                "curWave",
                "battleId",
                "magicCircle",
                "version",
                "isRecord",
                "episodeId",
                "fightActType",
                "lastChangeHeroUid",
                "progress",
                "progressMax",
                "fightTaskBox",
            ];
            let round_order = [
                "fightStep",
                "exPointInfo",
                "aiUseCards",
                "skillInfos",
                "beforeCards1",
                "teamACards1",
                "beforeCards2",
                "teamACards2",
                "nextRoundBeginStep",
                "useCardList",
                "heroSpAttributes",
                "actPoint",
                "isFinish",
                "moveNum",
                "power",
                "curRound",
                "lastChangeHeroUid",
            ];
            let side_order = [
                "entitys",
                "subEntitys",
                "skillInfos",
                "spEntitys",
                "indicators",
                "spFightEntities",
                "power",
                "clothId",
                "exTeamStr",
                "assistBoss",
                "assistBossInfo",
                "emitter",
                "emitterInfo",
                "playerEntity",
                "playerFinisherInfo",
                "energy",
                "cardHeat",
                "cardDeckSize",
                "bloodPool",
                "vorpalith",
                "itemSkillGroup",
                "heatScale",
                "musicInfo",
                "sub_entitys",
                "deadEntitys",
                "deadSubEntitys",
                "summons",
                "cardDeck",
                "cardDeckNum",
                "exPointInfos",
                "magicCircle",
                "powerInfos",
                "uid",
            ];
            let entity_order = [
                "buffs",
                "skillGroup1",
                "skillGroup2",
                "passiveSkill",
                "noEffectBuffs",
                "powerInfos",
                "act104EquipUids",
                "trialAct104Equips",
                "SummonedList",
                "summonedList",
                "equips",
                "uid",
                "modelId",
                "skin",
                "position",
                "entityType",
                "userId",
                "exPoint",
                "level",
                "currentHp",
                "attr",
                "exSkill",
                "shieldValue",
                "expointMaxAdd",
                "buffHarmStatistic",
                "equipUid",
                "trialEquip",
                "exSkillLevel",
                "baseAttr",
                "exSkillPointChange",
                "teamType",
                "enhanceInfoBox",
                "trialId",
                "career",
                "status",
                "guard",
                "subCd",
                "exPointType",
                "destinyStone",
                "destinyRank",
                "customUnitId",
            ];
            let attr_order = [
                "hp",
                "attack",
                "defense",
                "mdefense",
                "technic",
                "multiHpIdx",
                "multiHpNum",
            ];
            let equip_order = ["equipUid", "equipId", "equipLv", "refineLv"];
            let trial_equip_order = ["equipUid", "equipId", "equipLv", "refineLv"];
            let enhance_info_order = ["canUpgradeIds", "upgradedOptions", "uid"];
            let fight_step_order = [
                "actEffect",
                "actType",
                "fromId",
                "toId",
                "actId",
                "cardIndex",
                "supportHeroId",
                "fakeTimeline",
                "realSkillType",
                "realSkinId",
            ];
            let act_effect_order = [
                "cardInfoList",
                "fightTasks",
                "targetId",
                "effectType",
                "effectNum",
                "buff",
                "entity",
                "configEffect",
                "buffActId",
                "reserveId",
                "reserveStr",
                "summoned",
                "magicCircle",
                "cardInfo",
                "teamType",
                "fightStep",
                "assistBossInfo",
                "effectNum1",
                "emitterInfo",
                "playerFinisherInfo",
                "powerInfo",
                "cardHeatValue",
                "fight",
                "buffActInfo",
                "hurtInfo",
                "rouge2FightMusicInfo",
            ];
            let buff_order = [
                "actInfo",
                "buffId",
                "duration",
                "uid",
                "exInfo",
                "fromUid",
                "count",
                "actCommonParams",
                "layer",
                "type",
            ];
            let card_info_order = [
                "enchants",
                "extraInfos",
                "uid",
                "skillId",
                "cardEffect",
                "tempCard",
                "cardType",
                "heroId",
                "status",
                "targetUid",
                "extraInfo",
                "energy",
                "areaRedOrBlue",
                "heatId",
                "musicNote",
            ];
            let ex_point_info_order = ["powerInfos", "uid", "exPoint", "currentHp", "exPointType"];
            let hurt_info_order = [
                "damage",
                "reduceHp",
                "reduceShield",
                "careerRestraint",
                "critical",
                "assassinate",
                "hurtEffect",
                "damageFromType",
                "configEffect",
                "buffActId",
                "buffUid",
                "effectId",
                "skillId",
                "fromUid",
            ];

            if keys.contains("fight") && keys.contains("round") {
                reorder_object_with_keys(obj, &start_dungeon_reply_order);
            } else if keys.contains("attacker") && keys.contains("defender") {
                reorder_object_with_keys(obj, &fight_order);
            } else if keys.contains("playerEntity")
                || keys.contains("subEntitys")
                || keys.contains("sub_entitys")
                || (keys.contains("entitys") && keys.contains("clothId"))
            {
                reorder_object_with_keys(obj, &side_order);
            } else if keys.contains("uid")
                && keys.contains("modelId")
                && keys.contains("passiveSkill")
                && keys.contains("skillGroup1")
            {
                if obj.contains_key("summonedList")
                    && !obj.contains_key("SummonedList")
                    && let Some(v) = obj.remove("summonedList")
                {
                    obj.insert("SummonedList".to_string(), v);
                }

                reorder_object_with_keys(obj, &entity_order);
            } else if keys.contains("hp") && keys.contains("attack") && keys.contains("mdefense") {
                reorder_object_with_keys(obj, &attr_order);
            } else if keys.contains("equipUid")
                && keys.contains("equipId")
                && keys.contains("refineLv")
            {
                reorder_object_with_keys(obj, &equip_order);
            } else if keys.contains("canUpgradeIds")
                && keys.contains("upgradedOptions")
                && keys.contains("uid")
            {
                reorder_object_with_keys(obj, &enhance_info_order);
            } else if keys.contains("fightStep") && keys.contains("actPoint") {
                reorder_object_with_keys(obj, &round_order);
            } else if keys.contains("actEffect") && keys.contains("actType") {
                if let Some(v) = obj.get_mut("actType")
                    && let Some(n) = v.as_i64()
                {
                    let name = match n {
                        1 => Some("SKILL"),
                        2 => Some("BUFF"),
                        3 => Some("EFFECT"),
                        4 => Some("CHANGEHERO"),
                        5 => Some("CHANGEWAVE"),
                        _ => None,
                    };
                    if let Some(name) = name {
                        *v = Value::from(name);
                    }
                }
                if matches!(obj.get("cardIndex"), Some(Value::Null)) {
                    obj.insert("cardIndex".to_string(), Value::from(0));
                }
                if matches!(obj.get("supportHeroId"), Some(Value::Null)) {
                    obj.insert("supportHeroId".to_string(), Value::from(0));
                }
                if matches!(obj.get("fakeTimeline"), Some(Value::Null)) {
                    obj.insert("fakeTimeline".to_string(), Value::from(false));
                }
                if matches!(obj.get("realSkillType"), Some(Value::Null)) {
                    obj.insert("realSkillType".to_string(), Value::from(0));
                }
                if matches!(obj.get("realSkinId"), Some(Value::Null)) {
                    obj.insert("realSkinId".to_string(), Value::from(0));
                }
                reorder_object_with_keys(obj, &fight_step_order);
            } else if keys.contains("effectType")
                && (keys.contains("targetId")
                    || keys.contains("fightStep")
                    || keys.contains("effectNum"))
            {
                if matches!(obj.get("targetId"), Some(Value::Null)) {
                    obj.insert("targetId".to_string(), Value::from(0));
                }
                if matches!(obj.get("effectNum"), Some(Value::Null)) {
                    obj.insert("effectNum".to_string(), Value::from(0));
                }
                if matches!(obj.get("configEffect"), Some(Value::Null)) {
                    obj.insert("configEffect".to_string(), Value::from(0));
                }
                if matches!(obj.get("buffActId"), Some(Value::Null)) {
                    obj.insert("buffActId".to_string(), Value::from(0));
                }
                if matches!(obj.get("reserveId"), Some(Value::Null)) {
                    obj.insert("reserveId".to_string(), Value::from(0));
                }
                if matches!(obj.get("reserveStr"), Some(Value::Null)) {
                    obj.insert("reserveStr".to_string(), Value::from(""));
                }
                if matches!(obj.get("teamType"), Some(Value::Null)) {
                    obj.insert("teamType".to_string(), Value::from(0));
                }
                if matches!(obj.get("effectNum1"), Some(Value::Null)) {
                    obj.insert("effectNum1".to_string(), Value::from(0));
                }

                reorder_object_with_keys(obj, &act_effect_order);
            } else if keys.contains("buffId") && keys.contains("actInfo") {
                if matches!(obj.get("duration"), Some(Value::Null)) {
                    obj.insert("duration".to_string(), Value::from(0));
                }
                if matches!(obj.get("exInfo"), Some(Value::Null)) {
                    obj.insert("exInfo".to_string(), Value::from(0));
                }
                if matches!(obj.get("count"), Some(Value::Null)) {
                    obj.insert("count".to_string(), Value::from(0));
                }
                if matches!(obj.get("actCommonParams"), Some(Value::Null)) {
                    obj.insert("actCommonParams".to_string(), Value::from(""));
                }
                if matches!(obj.get("type"), Some(Value::Null)) {
                    obj.insert("type".to_string(), Value::from(0));
                }
                reorder_object_with_keys(obj, &buff_order);
            } else if keys.contains("skillId") && keys.contains("uid") {
                if let Some(v) = obj.get_mut("cardType")
                    && let Some(n) = v.as_i64()
                    && let Ok(kind) = <CardType as TryFrom<i32>>::try_from(n as i32)
                {
                    *v = Value::from(kind.as_str_name());
                }
                if let Some(v) = obj.get_mut("status")
                    && let Some(n) = v.as_i64()
                    && let Ok(kind) = <CardStatus as TryFrom<i32>>::try_from(n as i32)
                {
                    *v = Value::from(kind.as_str_name());
                }
                reorder_object_with_keys(obj, &card_info_order);
            } else if keys.contains("uid")
                && keys.contains("exPoint")
                && keys.contains("currentHp")
                && keys.contains("exPointType")
            {
                reorder_object_with_keys(obj, &ex_point_info_order);
            } else if keys.contains("damage")
                && keys.contains("hurtEffect")
                && keys.contains("damageFromType")
            {
                if let Some(v) = obj.get_mut("damageFromType")
                    && let Some(n) = v.as_i64()
                    && let Ok(kind) = <DamageFromType as TryFrom<i32>>::try_from(n as i32)
                {
                    *v = Value::from(kind.as_str_name());
                }
                reorder_object_with_keys(obj, &hurt_info_order);
            } else if keys.contains("equipUid")
                && keys.contains("equipId")
                && keys.contains("equipLv")
                && keys.contains("refineLv")
            {
                reorder_object_with_keys(obj, &trial_equip_order);
            }
        }
        _ => {}
    }
}
