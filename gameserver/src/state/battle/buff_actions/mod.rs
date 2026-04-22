pub mod add_passive_skills;
pub mod attr;
pub mod attr_replace;
pub mod ban_lost_life;
pub mod blood_pool_ex;
pub mod blood_value_use_skill;
pub mod bullet;
pub mod ex_point_overflow_bank;
pub mod halo;
pub mod heal;
pub mod hp;
pub mod lost_life;
pub mod monitor_continue;
pub mod nuodika_cast;
pub mod raspberry;
pub mod shield;

pub mod result;
pub mod use_skill_to_enemy;

pub use crate::state::battle::context::effect_context::EffectContext;
pub use heal::{heal, heal_by_two_attr};
pub use result::ActionResult;

use super::{skill::SkillExecutor, types::effects::EffectType};
use sonettobuf::ActEffect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureStage {
    /// Runs before BUFFADD is emitted.
    BeforeBuffAdd,
    /// Runs after BUFFADD is emitted.
    AfterBuffAdd,
}

/// Execution timing for a feature relative to BUFFADD emission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureTiming {
    AfterBuffAdd,
    BeforeAndAfterBuffAdd,
}

#[derive(Debug, Clone, Copy)]
struct FeatureSpec {
    /// Execution timing contract for this feature type.
    timing: FeatureTiming,
}

/// Central feature registry (first slice): maps buff_act.type -> execution timing.
fn feature_spec(act_type: &str) -> FeatureSpec {
    let timing = match act_type {
        "Attr" | "EachChangeAttr" | "LostHpCountAddBuff" => FeatureTiming::BeforeAndAfterBuffAdd,
        _ => FeatureTiming::AfterBuffAdd,
    };
    FeatureSpec { timing }
}

fn should_run_in_stage(timing: FeatureTiming, stage: FeatureStage) -> bool {
    match (timing, stage) {
        (FeatureTiming::AfterBuffAdd, FeatureStage::BeforeBuffAdd) => false,
        (FeatureTiming::AfterBuffAdd, FeatureStage::AfterBuffAdd) => true,
        (FeatureTiming::BeforeAndAfterBuffAdd, FeatureStage::BeforeBuffAdd) => true,
        (FeatureTiming::BeforeAndAfterBuffAdd, FeatureStage::AfterBuffAdd) => true,
    }
}

fn for_each_buff_feature(buff_id: i32, skip_first: bool, mut f: impl FnMut(&str, &[&str])) {
    // Parse and resolve configured feature entries once, then run caller-provided stage logic.
    let cfg = config::configs::get();
    let Some(buff) = cfg.skill_buff.iter().find(|b| b.id == buff_id) else {
        return;
    };
    if buff.features.is_empty() {
        return;
    }

    for entry in buff
        .features
        .split('|')
        .skip(if skip_first { 1 } else { 0 })
    {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts
            .first()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let act_type = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type.as_str())
            .unwrap_or("");

        f(act_type, &parts);
    }
}

fn run_before_add_feature(
    act_type: &str,
    parts: &[&str],
    ctx: &mut EffectContext,
    condition_id: i32,
) -> ActionResult {
    // Before-add stage only handles feature-specific pre-broadcast behavior.
    // Other features intentionally do nothing in this stage.
    match act_type {
        // HP Attr pre-broadcasts happen before BUFFADD for EnterFight/BattleStart.
        "Attr" => {
            let char_attr_id = parse_parts(parts, 1);
            let rate = parse_parts(parts, 2);
            // Attr(HP) pre-broadcast is an EnterFight/BattleStart behavior.
            // Keep it strict to avoid career/unconditional attr adds emitting extra pairs.
            if char_attr_id == 101 && (condition_id == 5 || condition_id == 5021) {
                let base_hp = ctx
                    .target_entity()
                    .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
                    .unwrap_or(0);
                let new_max = base_hp + base_hp * rate / 1000;
                let current_hp = ctx.target_hp();
                let mut effects = Vec::new();
                for _ in 0..2 {
                    effects.push(ActEffect {
                        effect_type: Some(EffectType::MaxHpChange as i32),
                        target_id: Some(ctx.target_uid()),
                        effect_num: Some(new_max),
                        ..Default::default()
                    });
                    effects.push(ActEffect {
                        effect_type: Some(EffectType::CurrentHpChange as i32),
                        target_id: Some(ctx.target_uid()),
                        effect_num: Some(current_hp),
                        ..Default::default()
                    });
                }
                ActionResult::effects(effects)
            } else {
                ActionResult::empty()
            }
        }
        // HP EachChangeAttr pre-broadcasts before BUFFADD; post emits None(0).
        "EachChangeAttr" => {
            let char_attr_id = parse_parts(parts, 1);
            let source_rate = parse_parts(parts, 4);
            if char_attr_id == 101 {
                let caster_max_hp = ctx
                    .caster_entity()
                    .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
                    .unwrap_or(0);
                let target_max_hp = ctx
                    .target_entity()
                    .and_then(|e| e.attr.as_ref().and_then(|a| a.hp))
                    .unwrap_or(0);
                let current_hp = ctx.target_hp();
                let new_max = target_max_hp + caster_max_hp * source_rate / 1000;
                ActionResult::effects(vec![
                    ActEffect {
                        effect_type: Some(EffectType::MaxHpChange as i32),
                        target_id: Some(ctx.target_uid()),
                        effect_num: Some(new_max),
                        ..Default::default()
                    },
                    ActEffect {
                        effect_type: Some(EffectType::CurrentHpChange as i32),
                        target_id: Some(ctx.target_uid()),
                        effect_num: Some(current_hp),
                        ..Default::default()
                    },
                ])
            } else {
                ActionResult::empty()
            }
        }
        "LostHpCountAddBuff" => {
            let child_buff_id = parse_parts(parts, 1);
            let mut result = hp::lost_hp_count_add_buff(ctx, child_buff_id);
            result
                .effects
                .retain(|e| matches!(e.effect_type, Some(108) | Some(109)));
            result
        }
        _ => ActionResult::empty(),
    }
}

fn run_after_add_feature(
    act_type: &str,
    parts: &[&str],
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    buff_id: i32,
    has_bloodpool: bool,
) -> ActionResult {
    // LostHpCountAddBuff is split across stages:
    // - before: HP broadcasts (108/109)
    // - after: trailing marker only (None/0)
    if act_type == "LostHpCountAddBuff" {
        let child_buff_id = parse_parts(parts, 1);
        let mut result = hp::lost_hp_count_add_buff(ctx, child_buff_id);
        // Keep post-pass marker only; HP broadcasts are emitted in pre-pass.
        result
            .effects
            .retain(|e| e.effect_type == Some(EffectType::None as i32));
        return result;
    }

    dispatch_feature(act_type, parts, ctx, executor, buff_id, has_bloodpool)
}

fn apply_action_result(
    all_effects: &mut Vec<ActEffect>,
    result: ActionResult,
    executor: Option<&mut SkillExecutor>,
) {
    // Shared fan-out so before/after pipelines merge ActionResult the same way.
    all_effects.extend(result.effects);
    if let Some(executor) = executor {
        executor.side_effects.extend(result.side_effects);
        executor.pending_buff_dels.extend(result.buff_dels);
        executor
            .pending_monitor_triggers
            .extend(result.monitor_triggers);
    }
}

pub fn dispatch_feature(
    act_type: &str,
    parts: &[&str],
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    buff_id: i32,
    _has_bloodpool: bool,
) -> ActionResult {
    match act_type {
        "Attr" => attr::on_apply(ctx),
        "EachChangeAttr" => ActionResult::none(ctx.target),
        "AttrFromEntity" => attr::from_entity(ctx, buff_id),
        "AttrOnlyCalDamageReplaceAttr" | "AttrOnlyCalDamageReplaceAttrADCreator" => {
            ActionResult::empty()
        }

        "MasterHalo" => {
            let slave_buff_id = parse_parts(parts, 2);
            halo::master(ctx, executor, slave_buff_id)
        }
        "SlaveHalo" => ActionResult::empty(),

        "LostHpCountAddBuff" => {
            let child_buff_id = parse_parts(parts, 1);
            hp::lost_hp_count_add_buff(ctx, child_buff_id)
        }
        "CureUpByLostHp" => heal::cure_up_by_lost_hp(ctx),
        "Revive" => heal::revive(ctx),

        "Shield" => {
            let permille = parse_parts(parts, 3);
            shield::apply(ctx, permille)
        }
        "Rebound" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::Rebound as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "AddToTarget" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::AddToTarget as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),

        "Raspberry" => ActionResult::none(ctx.target),
        "RaspberryBigSkill" => ActionResult::empty(),
        "MonitorContinueChannel" => ActionResult::empty(),

        "MonsterLabel" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::MonsterLabelBuff as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "ExPointOverflowBank" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::ExPointOverflowBank as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "ExPointMaxAdd" => {
            let amount = parse_parts(parts, 1);
            ActionResult::single(ActEffect {
                effect_type: Some(EffectType::ExPointMaxAdd as i32),
                target_id: Some(ctx.target),
                effect_num: Some(amount),
                ..Default::default()
            })
        }
        "TeammateInjuryCount" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::TeammateInjuryCount as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "PoisonSettleCanCrit" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::PoisonSettleCanCrit as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "RealHurtFix" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::RealHurtFix as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "RealHarmFix" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::RealHarmFix as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "RealHurtSkillEffectFix" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::RealHurtSkillEffectFix as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),
        "RealHarmSkillEffectFix" => ActionResult::single(ActEffect {
            effect_type: Some(EffectType::RealHarmSkillEffectFix as i32),
            target_id: Some(ctx.target),
            effect_num: Some(0),
            ..Default::default()
        }),

        // No-op at application time
        "FixAttrBySubBuffLayer"
        | "AddPassiveSkills"
        | "SubBuff"
        | "Bullet"
        | "CreateMaxHpAdditionalDamageAndRemove"
        | "LifeAttackFixRate"
        | "AddBuffByOtherExSkill"
        | "ProbabilityAddBuff"
        | "Poison" => ActionResult::none(ctx.target),

        _ => ActionResult::none(ctx.target),
    }
}

pub fn apply_before_buff_add_features(
    ctx: &mut EffectContext,
    buff_id: i32,
    condition_id: i32,
) -> Vec<ActEffect> {
    // Stage pipeline:
    // 1) iterate configured features
    // 2) filter by timing contract
    // 3) run before-stage handler
    // 4) merge emitted effects
    let mut effects = Vec::new();
    for_each_buff_feature(buff_id, false, |act_type, parts| {
        let spec = feature_spec(act_type);
        if !should_run_in_stage(spec.timing, FeatureStage::BeforeBuffAdd) {
            return;
        }
        let result = run_before_add_feature(act_type, parts, ctx, condition_id);
        apply_action_result(&mut effects, result, None);
    });

    effects
}

pub fn apply_after_buff_add_features(
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    buff_id: i32,
    has_bloodpool: bool,
) -> Vec<ActEffect> {
    let cfg = config::configs::get();
    // Legacy compatibility: feature id 772 is metadata-only at add-time;
    // skip first entry in post stage.
    let skip_first = cfg
        .skill_buff
        .iter()
        .find(|b| b.id == buff_id)
        .map(|buff| {
            buff.features
                .split('|')
                .next()
                .map(|s| s.split('#').next().unwrap_or("") == "772")
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let mut effects = Vec::new();
    for_each_buff_feature(buff_id, skip_first, |act_type, parts| {
        let spec = feature_spec(act_type);
        if !should_run_in_stage(spec.timing, FeatureStage::AfterBuffAdd) {
            return;
        }
        let result = run_after_add_feature(act_type, parts, ctx, executor, buff_id, has_bloodpool);
        apply_action_result(&mut effects, result, Some(executor));
    });

    effects
}

fn parse_parts(parts: &[&str], idx: usize) -> i32 {
    parts
        .get(idx)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}
