mod action;
pub mod add_buff_both;
pub mod add_passive_skills;
pub mod attr;
pub mod attr_replace;
pub mod ban_lost_life;
pub mod blood_pool_ex;
pub mod blood_value_use_skill;
mod bootstrap;
pub mod bullet;
pub mod ex_point_overflow_bank;
pub mod halo;
pub mod heal;
pub mod hp;
pub mod injury_bank;
pub mod lost_life;
mod markers;
pub mod monitor_continue;
mod no_op;
pub mod nuodika;
pub mod nuodika_cast;
pub mod probability_add_buff;
pub mod raspberry;
pub mod round_end;
pub mod shield;

use self::action::{BUFF_ACTION_REGISTRY, BUFF_HANDLER_REGISTRY, BuffActCtx, BuffStage};

pub mod result;
pub mod use_skill_to_enemy;

pub use crate::state::battle::context::effect_context::EffectContext;
pub use heal::heal;
pub use result::ActionResult;

use super::skill::SkillExecutor;
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

/// Walk a buff's `features` string and return the parts of the entry
/// whose resolved `buff_act.type` equals `act_type`. Returns the raw
/// `[act_id_str, arg1, arg2, ...]` slice for the matching feature, or
/// `None` if the buff isn't configured / has no matching feature.
///
/// Use this for buff_act-specific parsers that all share the same
/// "find my feature, return its args" lookup. Each caller can then
/// `.get(N)?.trim().parse().ok()` for whichever index it cares about.
pub fn find_feature_parts(buff_id: i32, act_type: &str) -> Option<Vec<&'static str>> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&'static str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let matches = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type == act_type)
            .unwrap_or(false);
        if matches { Some(parts) } else { None }
    })
}

/// Convenience: like [`find_feature_parts`] but returns just the i32
/// at `param_idx`. Most single-value parsers reduce to one call here.
pub fn feature_param_i32(buff_id: i32, act_type: &str, param_idx: usize) -> Option<i32> {
    find_feature_parts(buff_id, act_type)?
        .get(param_idx)?
        .trim()
        .parse()
        .ok()
}

/// Walk a buff's features in order, calling `f(act_type, parts)` for
/// each. Returns the first non-`None` value the closure produces (so
/// callers preserve feature-order semantics across multiple
/// candidate act_types).
pub fn first_feature_match<T>(
    buff_id: i32,
    mut f: impl FnMut(&str, &[&str]) -> Option<T>,
) -> Option<T> {
    let cfg = config::configs::get();
    let buff = cfg.skill_buff.iter().find(|b| b.id == buff_id)?;
    buff.features.split('|').find_map(|entry| {
        let parts: Vec<&str> = entry.split('#').collect();
        let act_id: i32 = parts.first()?.trim().parse().ok()?;
        let act_type = cfg
            .buff_act
            .iter()
            .find(|a| a.id == act_id)
            .map(|a| a.r#type.as_str())?;
        f(act_type, &parts)
    })
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
    executor: &mut SkillExecutor,
    condition_id: i32,
) -> ActionResult {
    // Walk the per-handler registry first (parse → execute → steps
    // pattern). Then fall through to the legacy cluster registry for
    // the act_types that haven't migrated yet. Returns empty if
    // neither claims it (most act_types don't emit pre-stage).
    let mut buff_ctx = BuffActCtx {
        effect_ctx: ctx,
        executor,
        buff_id: 0,
        condition_id,
        has_bloodpool: false,
    };
    for handler in BUFF_HANDLER_REGISTRY {
        if let Some(result) = handler.run(act_type, BuffStage::BeforeBuffAdd, parts, &mut buff_ctx)
        {
            return result;
        }
    }
    for cluster in BUFF_ACTION_REGISTRY {
        if let Some(result) =
            cluster.execute(act_type, parts, &mut buff_ctx, BuffStage::BeforeBuffAdd)
        {
            return result;
        }
    }
    ActionResult::empty()
}

fn run_after_add_feature(
    act_type: &str,
    parts: &[&str],
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
    buff_id: i32,
    has_bloodpool: bool,
) -> ActionResult {
    // The post-stage path used to special-case LostHpCountAddBuff
    // before deferring to dispatch_feature; with the registry the
    // single dispatcher handles every variant uniformly.
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
    let target = ctx.target;
    let mut buff_ctx = BuffActCtx {
        effect_ctx: ctx,
        executor,
        buff_id,
        condition_id: 0,
        has_bloodpool: _has_bloodpool,
    };
    // Walk the per-handler registry first (parse → execute → steps
    // pattern). Falls through to the legacy cluster registry for
    // unmigrated act_types.
    for handler in BUFF_HANDLER_REGISTRY {
        if let Some(result) = handler.run(act_type, BuffStage::AfterBuffAdd, parts, &mut buff_ctx) {
            return result;
        }
    }
    for cluster in BUFF_ACTION_REGISTRY {
        if let Some(result) =
            cluster.execute(act_type, parts, &mut buff_ctx, BuffStage::AfterBuffAdd)
        {
            return result;
        }
    }
    // No cluster or handler claimed this act_type — emit the legacy
    // None(0) placeholder so the dispatcher's feature loop accounts
    // for the slot.
    ActionResult::none(target)
}

pub fn apply_before_buff_add_features(
    ctx: &mut EffectContext,
    executor: &mut SkillExecutor,
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
        let result = run_before_add_feature(act_type, parts, ctx, executor, condition_id);
        apply_action_result(&mut effects, result, Some(executor));
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
