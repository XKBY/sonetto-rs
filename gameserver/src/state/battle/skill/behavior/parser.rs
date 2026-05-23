use super::super::super::BehaviorType;
use config::configs;

pub fn parse_behavior(raw: &str) -> BehaviorType {
    let cfg = configs::get();
    if raw.is_empty() {
        return BehaviorType::Unknown {
            raw: raw.to_string(),
        };
    }

    let parts: Vec<&str> = raw.split('#').collect();
    let id: i32 = parts[0].parse().unwrap_or(0);
    let p1: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    let p2: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);

    // Recoleta temp-card behavior in live captures.
    // Keep this ID-based fallback stable even if behavior type rows drift across data bundles.
    if id == 60175 {
        return BehaviorType::DirectUseBigSkill;
    }
    // Live data bundles can label 60010 as DisperseForce2, but runtime payloads
    // use 60010#<buff_id>[#count] as an AddBuff lane.
    if id == 60010 {
        return BehaviorType::AddBuff {
            buff_id: p1,
            count: p2,
        };
    }
    if id == 60039 {
        return BehaviorType::RealDamageSelfAndAddBuffToTarget {
            amount_permille: p1,
            buff_id: p2,
        };
    }
    // Kakania's EX `Id, Ego and Superego` (skill 30800131,
    // `behavior1 = 60040#10000#1#0`) is the consume-and-bonus version
    // of `60038`: same Genesis-bonus formula, but the caster's
    // stored Empathy is reset to 0 once the bonus has been computed.
    // Per the in-game ability text: "1-target attack. Deals X% Mental
    // DMG plus (Current [Empathy] × multiplier%) Genesis DMG to the
    // target, resets [Empathy] to zero, and then starts recording
    // the damage the target takes for the round."
    if id == 60040 {
        return BehaviorType::ConsumeInjuryBankAndDamage {
            multiplier_permille: p1,
        };
    }
    // `Disperse2` (skill_behavior id 30009) is a single-buff drop —
    // e.g. Sentinel `31260131 slot4 = '30009#31260121'`. The
    // type-name wildcard below (`starts_with("Disperse")`) catches
    // it as the argless `Disperse` (drop every buff), discarding
    // the buff_id. Route this specific id to the existing
    // `DisperseForce` runtime to preserve the targeted-buff
    // semantic. Other Disperse-family ids (e.g. 30003 / 30004 /
    // 30008 / 30016 / 30017 / 90002) keep the legacy mapping until
    // we have evidence each carries a buff_id arg LIVE-side; the
    // wildcard route already widens enough to be wrong if their
    // semantics also differ — addressed one id at a time.
    if id == 30009 && p1 > 0 {
        return BehaviorType::DisperseForce { buff_id: p1 };
    }
    // `CritRateAlter2` (skill_behavior id 60228) bumps the caster's
    // Crit Rate (Attr::Cri = 201). LIVE encodes it as `60228#<permille>`
    // — e.g. Sentinel `31260181 slot2 = '60228#800'` (+80% Cri while
    // her Hour of Repentance buff 31260151 is up). Route to the
    // existing `AttrFix` runtime: the Cri attr is one of the
    // attribute slots `executor.add_attr_bonus` already updates.
    // Note: id 100023 is also tagged `CritRateAlter2` in the data
    // bundle but is not in current fixtures — leaving it
    // unaliased until we see a LIVE use.
    if id == 60228 {
        return BehaviorType::AttrFix {
            attr_id: crate::state::battle::types::attr::AttrId::Cri as i32,
            amount: p1,
        };
    }
    // `AttrFixByLoseHp` (skill_behavior id 60033) is encoded as
    // `60033#<step_permille>#<attr_id>#<bonus_per_stack>#<max_stacks>`.
    // Semmelweis Insight III 308801821 slot 6 carries
    // `60033#100#205#75#8` — i.e. for each 10% of MaxHP missing
    // on the caster, grant +7.5% AddDmg (attr 205), capped at 8
    // stacks (60% total). The `AttrFix` wildcard below would catch
    // this by name and only read the first two args, so we route
    // by id before the wildcard.
    if id == 60033 {
        let step_permille = p1;
        let attr_id = p2;
        let bonus_per_stack = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let max_stacks = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        return BehaviorType::AttrFixByLoseHp {
            step_permille,
            attr_id,
            bonus_per_stack,
            max_stacks,
        };
    }
    // `SettleDotAndCostDotDuration` (skill_behavior id 60073) — fires on
    // each enemy carrying skill 30980151 as a round-start passive (delivered
    // via `magic_circle 22100003.enemy_skills`). Per the in-game text on
    // 30980151: "At the start of the round, resolve 1 round of [Poison]
    // effects." The carrier walks its own Poison/DeadlyPoison buffs, deals
    // `caster.atk × permille / 1000` Genesis damage per stack, and
    // decrements `duringTime` by `rounds` — except when the carrier also
    // holds a `LockPoison(810)` buff (Tuesday's 30980131 lock-duration
    // debuff also applied by the array via `enemy_buff`), in which case the
    // damage still emits but `duringTime` stays pinned. LIVE encodes 60073
    // as `60073#<rounds>` (30980151 slot 1 = `60073#1` = 1 round per tick).
    if id == 60073 {
        return BehaviorType::SettleDotAndCostDotDuration { rounds: p1 };
    }
    // `PoisonConvertToTargetBuff` (skill_behavior id 60110) — Willow's
    // basic1 `Hag's Bane Pose` (skill 31040113) carries
    // `60110#<cap>#<buff_id>` per skill_effect (e.g. `60110#5#31040013`
    // for Lv.3). Per the in-game text: "1-target attack. Deals X% Mental
    // DMG; if the target hit already has an instance of [Poison],
    // convert 1 instance of [Poison] into 1 stack of [Hag's Bane] that
    // lasts 2 rounds; up to N instances of Poison can be converted by
    // this effect." LIVE-side the BuffAdd actEffect carries layer = cap,
    // duration = skill_buff.duringTime — no explicit Poison removal
    // event, the engine just stamps the buff. Argument order swaps the
    // `AddBuff` convention (`buff_id#count`) so route by id and re-pack
    // into the existing AddBuff runtime.
    if id == 60110 {
        return BehaviorType::AddBuff {
            buff_id: p2,
            count: p1,
        };
    }
    // `OriginDamageByAttrAndBuffGroupSize` (skill_behavior id 60127)
    // encodes a bonus damage emission of
    // `caster.attr[attr_id] × permille × buff_group_stacks_on_target / 1000`.
    // Tuesday's Lock-Sound mass attack carries
    // `30980131 slot 2 = '60127#1#102#300#7'` —
    // `caster ATK × 30% × Poison stacks on target` per her in-game text.
    if id == 60127 {
        let mode = p1;
        let attr_id = p2;
        let permille = parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let group_id = parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0);
        return BehaviorType::OriginDamageByAttrAndBuffGroupSize {
            mode,
            attr_id,
            permille,
            group_id,
        };
    }
    // Some live data uses 20021#<baseSkillId>#<rank> to direct-cast a derived skill id.
    // Keep AddBuffRanId behavior for true buff pools (small ids), but route skill-like ids.
    if id == 20021 && p1 >= 10000 {
        return BehaviorType::DirectUseGroupAndStarSkill {
            group: p1,
            rank: p2,
        };
    }
    let behavior_type = cfg
        .skill_behavior
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.r#type.as_str())
        .unwrap_or("");

    match behavior_type {
        "Damage" | "Damage2" | "Detonate" | "OriginDamage" | "OriginDamage2" => {
            BehaviorType::Damage { rate: p1 }
        }
        "Detonate2" => BehaviorType::Detonate2 {
            rate: p1,
            granted_buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        // Kakania's Empathy Genesis bonus family. Both Subconscious's
        // basic (`60038#multiplier`) and the Insight III heal-trigger
        // reactive (skill 30800161/2/3 with `60052#multiplier`) share
        // the same skill_behavior `type`. Per the in-game ability
        // description for Subconscious: "1-target attack. Deals X%
        // Mental DMG plus (Current [Empathy] × multiplier%) Genesis
        // DMG." The Insight III variant fires the same bonus
        // emission off a heal trigger via the standard
        // OriginDamageFromInjuryBank path. Multiplier is permille
        // (1800 / 2200 / 2600 / 1000 / 1200 across Lv1-3 + Insight
        // ranks).
        "OriginDamageFromInjuryBankBuff" => BehaviorType::OriginDamageFromInjuryBank {
            multiplier_permille: p1,
        },
        "Heal" => BehaviorType::Heal { rate: p1 },
        // `HealCantCrit` (ids 20012 / 20016 / 20018) encodes as
        // `act_id#?#attr_id#permille` and is sibling-emitted at the
        // parent step in LIVE — see the variant doc on
        // `BehaviorType::HealCantCrit`. Routed to a no-op here so the
        // host-wrapper executor stops emitting a wrong-target `et=4
        // num=1` leak; correct sibling-emission lives in future work.
        "HealCantCrit" => BehaviorType::HealCantCrit {
            attr_id: p2,
            permille: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "HealByTwoAttr" => BehaviorType::HealByTwoAttr {
            missing_percent: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            caster_hp_percent: parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddBuff" | "AddBuffRound" | "AddBuffRound2" => BehaviorType::AddBuff {
            buff_id: p1,
            count: p2,
        },
        "CatapultBuff" => BehaviorType::CatapultBuff {
            primary_stacks: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            buff_id: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            catapult_stacks: parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
            catapult_cap: parts.get(6).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddTargetBuffByPoison" => BehaviorType::AddTargetBuffByPoison {
            stack_count: parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_targets: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "CreateAdditionalDamageAddBuff" => BehaviorType::AddBuff {
            buff_id: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            count: 0,
        },
        "ConsumeBloodAddBuff" => BehaviorType::ConsumeBloodAddBuff {
            consume: p1,
            buff_id: p2,
            count: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "ConsumeBloodAddBuff2" => BehaviorType::ConsumeBloodAddBuff2 {
            consume: p1,
            buff_id: p2,
            count: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "AddExPoint" | "AttrFixExPoint" => {
            if id == 20002 {
                BehaviorType::AddExPointWithMax { amount: p1 }
            } else {
                BehaviorType::AddExPoint { amount: p1 }
            }
        }
        "LostLife" => BehaviorType::LostLife {
            mode: p1,
            attr_id: p2,
            permille: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            behavior_id: id,
        },
        "Bloodlust" => BehaviorType::Bloodlust { amount: p1 },
        "AverageLife" => BehaviorType::AverageLife,
        "BloodPoolValueChange" => BehaviorType::BloodPoolValueChange { amount: p1 },
        "BloodPoolMaxChange" => BehaviorType::BloodPoolMaxChange { amount: p1 },
        "AttrModify" => BehaviorType::AttrModify {
            attr_id: p1,
            amount: p2,
        },
        "BeAttackedAssassinate" => BehaviorType::BeAttackedAssassinate {
            attr_id: p1,
            amount: p2,
        },
        "ConsumeBuffByTypeId" => BehaviorType::ConsumeBuffByTypeId {
            type_id: p1,
            count: p2,
        },
        t if t.starts_with("DisperseForce") => BehaviorType::DisperseForce { buff_id: p1 },
        t if t.starts_with("Disperse") => BehaviorType::Disperse,
        t if t.starts_with("Purify") => BehaviorType::Purify,
        "ChangePower" => BehaviorType::ChangePower { amount: p1 },
        t if t.starts_with("AttrFix") => {
            // Most AttrFix-like behaviors are encoded as:
            //   behavior_id#attr_id#amount
            // Keep the parser permissive because some variants append extra params.
            let attr_id = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(p1);
            let amount = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(p2);
            BehaviorType::AttrFix { attr_id, amount }
        }
        "SkillRateUp" => BehaviorType::SkillRateUp { rate: p1 },
        "ConsumePowerDirectUseSkill" => BehaviorType::ConsumePowerDirectUseSkill {
            count: p1,
            skill_id: p2,
        },
        "DirectUseSkill" => BehaviorType::DirectUseSkill { skill_id: p1 },
        "DirectUseBigSkill" => BehaviorType::DirectUseBigSkill,
        "ConsumeExPointAddAttr" => BehaviorType::ConsumeExPointAddAttr {
            min_consume: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            max_consume: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "SkillRateUpBySelfBuffType" => BehaviorType::SkillRateUpBySelfBuffType {
            buff_type_id: p1,
            rate: p2,
        },
        "SkillRateUpBuffType" => BehaviorType::SkillRateUpByBuffType {
            rate: p1,
            buff_types: parts
                .iter()
                .skip(3)
                .filter_map(|v| v.parse().ok())
                .collect(),
        },
        "RandomUseSkill" => BehaviorType::RandomUseSkill {
            raw: raw.to_string(),
        },
        "MonsterChange" => BehaviorType::MonsterChange {
            new_monster_id: p1,
            probability_permille: p2,
        },
        "Kill" => BehaviorType::Kill,
        "Summon" => BehaviorType::Summon { skill_id: p1 },
        "RaspberryAddCount" => BehaviorType::RaspberryAddCount {
            attr_id: p1,
            rate: p2,
        },
        "AddBuffRanId" => BehaviorType::AddBuffRanId {
            pool_buff_id: p1,
            count: p2,
        },
        "AddMagicCircle" | "MagicCircleAddRound" => BehaviorType::AddMagicCircle { circle_id: p1 },
        "MagicCircleAttr" => {
            // Encoding: `60076#side#attr#permille[#side2#attr2#permille2]...`.
            // `parts[0]` is the behavior id (60076); subsequent parts come
            // in (side, attr, permille) triples. `side`: 1 = caster's team,
            // 2 = opposing team.
            let mut modifiers = Vec::new();
            let mut i = 1;
            while i + 2 < parts.len() {
                let side: i32 = parts.get(i).and_then(|v| v.parse().ok()).unwrap_or(0);
                let attr_id: i32 = parts.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                let permille: i32 = parts.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0);
                if attr_id != 0 {
                    modifiers.push((side, attr_id, permille));
                }
                i += 3;
            }
            BehaviorType::MagicCircleAttr { modifiers }
        }
        "DirectUseGroupAndStarSkill" => BehaviorType::DirectUseGroupAndStarSkill {
            group: p1,
            rank: p2,
        },
        "ReplaceBuff2" => BehaviorType::ReplaceBuff2 {
            source_buff_ids: parts
                .get(1)
                .map(|p| p.split(',').filter_map(|v| v.parse().ok()).collect())
                .unwrap_or_default(),
            replacement_buff_id: parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            duration: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            count: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(1),
        },
        "CrystalAddCard" => BehaviorType::CrystalAddCard,

        "ShellUseSkill" => BehaviorType::ShellUseSkill {
            group: p1,
            skill_id: p2,
        },
        "ShellAssign" => BehaviorType::ShellAssign {
            slot: p1,
            skill_id: p2,
        },
        "PurifyX" => BehaviorType::PurifyX {
            type_ids: parts[1..].iter().filter_map(|v| v.parse().ok()).collect(),
        },
        "IgnoreSkillConfigDamageRate" => BehaviorType::IgnoreSkillConfigDamageRate,

        "LostAllLifeByAttr" => BehaviorType::LostAllLifeByAttr {
            caster_attr: p1,
            caster_amount: p2,
            target_attr: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            target_amount: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        },

        "DamageRealLostLife" => BehaviorType::DamageRealLostLife {
            buff_id: p1,
            duration: p2,
            rate: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        "NuoDiKaDamage" => BehaviorType::NuoDiKaDamage {
            primary_buff_id: p1,
            primary_rate: p2,
            secondary_buff_id: parts.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
            secondary_rate: parts.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            self_loss_param: parts.get(5).and_then(|v| v.parse().ok()).unwrap_or(0),
        },
        _ => {
            if !behavior_type.is_empty() {
                //tracing::warn!("Unhandled behavior type: {} (id={})", behavior_type, id);
            }
            BehaviorType::Unknown {
                raw: raw.to_string(),
            }
        }
    }
}
