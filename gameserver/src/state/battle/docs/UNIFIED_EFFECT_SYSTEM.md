# Unified Effect System

Design spec for collapsing `BehaviorAction`, `BuffActionHandler`, passive conditions,
and `TriggerPass` into a single data-driven reactive system with composable action pipelines.

**Goals:**
- Data-driven extensibility: common mechanics defined entirely in config (no new Rust code).
- Deduplication: one dispatch mechanism replaces three structurally identical registries.
- Exotic mechanics remain hand-coded Rust behind the same unified trait.

**Migration strategy:** Incremental coexistence — new system lives alongside old, handlers migrate one at a time.

---

## Core Data Model

The unified primitive is an `Effect`:

```rust
struct Effect {
    id: EffectId,
    trigger: TriggerKind,             // WHEN it fires
    condition: Option<ConditionExpr>, // IF gate (composable tree)
    actions: Vec<ActionStep>,         // WHAT happens (pipeline, executed in order)
    target_selector: TargetSelector,  // WHO it applies to
    phase: PhaseFilter,               // which battle phase it's valid in
}
```

### TriggerKind

Flat enum covering all trigger points (replaces `BuffStage` + `TriggerEvent` fields + `PhaseFilter` checks):

```rust
enum TriggerKind {
    // Combat
    OnHit,
    OnCriticalHit,
    OnKill,
    OnBeAttacked,
    OnTakeDamage,
    OnDealDamage,
    // Buff lifecycle
    OnBuffAdd,
    OnBuffRemove,
    OnDeath,
    OnRevive,
    // Resource
    OnGainShield,
    OnShieldBreak,
    OnExChange,
    OnBloodpoolChange,
    // Round
    OnRoundStart,
    OnRoundEnd,
    OnCast,
    OnPostSkill,
    // Phase
    OnBattleStart,
    OnEnterFight,
    OnWaveChange,
}
```

### ActionStep

One unit in the composable pipeline. Each step receives the output context of the previous step:

```rust
enum ActionStep {
    DealDamage { base_param: i32, element: Element },
    Heal { base_param: i32 },
    AddBuff { buff_id: i32, duration: i32, stacks: i32 },
    RemoveBuff { buff_id: i32 },
    ModifyAttr { attr_id: i32, value: i32, mode: AttrMode },
    Shield { amount: i32, element: Element },
    DotTick { dot_type: DotType, base_param: i32 },
    ExChange { delta: i32 },
    BloodpoolChange { delta: i32, team: i32 },
    FireSkill { skill_id: i32 },          // re-enter SkillExecutor
    Custom { handler_name: &'static str }, // exotic hand-coded logic
}
```

### ConditionExpr

Replaces the current flat `ConditionType` enum with a composable tree:

```rust
enum ConditionExpr {
    And(Vec<ConditionExpr>),
    Or(Vec<ConditionExpr>),
    Not(Box<ConditionExpr>),
    Leaf(ConditionLeaf),
}

enum ConditionLeaf {
    HpBelow(i32),          // percent
    HpAbove(i32),
    HasBuff(i32),          // buff_id
    BuffStacksGe(i32, i32), // buff_id, min_stacks
    IsElement(Element),
    ActOrder(Range<i32>),
    TargetCount(CmpOp, i32),
    UsedExSkill,
    ActiveUseSkill,
    ActiveUseSkillId(i32),
    // ... extensible
}
```

### TargetSelector

```rust
enum TargetSelector {
    Self_,
    PrimaryTarget,
    AllEnemies,
    AllAllies,
    RandomEnemy(usize),
    RandomAlly(usize),
    LowestHpAlly,
    HighestHpEnemy,
    Custom { resolver_name: &'static str },
}
```

---

## Dispatch & Interpreter

### EffectEngine

Single dispatch loop replaces `BEHAVIOR_REGISTRY`, `BUFF_HANDLER_REGISTRY`, and `TriggerPass` iteration:

```rust
struct EffectEngine {
    /// All registered effects, indexed by trigger kind for O(1) lookup
    effects_by_trigger: HashMap<TriggerKind, Vec<Effect>>,
    /// Hand-coded handlers for Custom action steps
    custom_handlers: HashMap<&'static str, Box<dyn CustomActionHandler>>,
}

impl EffectEngine {
    /// Main entry point — replaces all three dispatch systems
    fn emit(&mut self, event: BattleEvent, ctx: &mut EffectCtx) -> Vec<ActEffect> {
        let trigger = event.trigger_kind();
        let candidates = self.effects_by_trigger.get(&trigger);
        let mut results = Vec::new();
        for effect in candidates {
            if !effect.condition_met(ctx) { continue; }
            let targets = effect.target_selector.resolve(ctx);
            for target in targets {
                results.extend(self.run_pipeline(&effect.actions, ctx, target));
            }
        }
        results
    }

    fn run_pipeline(
        &mut self,
        actions: &[ActionStep],
        ctx: &mut EffectCtx,
        target: i64,
    ) -> Vec<ActEffect> {
        let mut output = Vec::new();
        for step in actions {
            match step {
                ActionStep::Custom { handler_name } => {
                    let handler = self.custom_handlers.get(handler_name);
                    output.extend(handler.execute(ctx, target));
                }
                _ => output.extend(self.interpret_action(step, ctx, target)),
            }
        }
        output
    }
}
```

### Key Properties

- **One event type** — `BattleEvent` replaces `TriggerEvent` + `BuffStage` + `PhaseFilter` checks. Carries all context (caster, target, skill_id, damaged_uids, etc.) in one struct.
- **Lazy condition evaluation** — `condition_met` only evaluates when the trigger matches. The pre-filter optimization (current `has_combat_reactive_condition`) becomes an internal detail.
- **Pipeline context threading** — Each `ActionStep` reads/writes a `PipelineCtx` (accumulated damage, last buff added, etc.) so later steps can reference earlier results (e.g. "add buff with stacks = damage dealt / 100").
- **Recursion guard** — `FireSkill` and `Custom` handlers that re-enter `emit` share the existing `call_depth` cap (64).

### Custom Handler Trait

For exotic mechanics that can't be expressed as config:

```rust
trait CustomActionHandler {
    fn execute(&self, ctx: &mut EffectCtx, target: i64) -> Vec<ActEffect>;
}
```

Registered by name, invoked via `ActionStep::Custom { handler_name: "nuodika_damage" }`.

---

## Migration Phases

### Phase 1: Foundation (no behavior change)

- Define types in new `gameserver/src/state/battle/effect/` module:
  - `Effect`, `TriggerKind`, `ActionStep`, `ConditionExpr`, `TargetSelector`
  - `EffectEngine` with `emit()` and `run_pipeline()`
- Implement interpreter for common `ActionStep` variants (damage, heal, add_buff, remove_buff, modify_attr, shield, ex_change).
- Wire `EffectEngine` into `FightContext` (owned alongside `Managers` and `Mechanics`).
- **No existing code changes.** Engine exists but nothing calls it yet.

### Phase 2: Config Loader

- Parser reads existing config tables and produces `Vec<Effect>`:
  - `skill_effect` rows → `Effect` with trigger from `effectTime`, condition from condition columns, actions from `BehaviorType` + params.
  - `skill_buff` + `buff_act` rows → `Effect` with trigger from `buff_act.effectTime`, actions from `buff_act.type` + feature params.
  - Passive skills → `Effect` with trigger inferred from condition type (e.g. `ActiveUseSkill` → `OnCast`).
- Map current `effectTime` values → `TriggerKind`.
- Map current condition strings → `ConditionExpr` tree.
- Map current `behavior_type` / `buff_act.type` + params → `Vec<ActionStep>` pipeline.
- Load into `EffectEngine::effects_by_trigger` at battle init.

### Phase 3: Migrate Common Data-Driven Handlers

Migrate in order of simplicity and volume:

| # | Handler | Old location | New ActionStep |
|---|---------|-------------|----------------|
| 1 | Attr / EachChangeAttr | `buff_actions/attr.rs` | `ModifyAttr` |
| 2 | AddBuff | `skill/behavior/add_buff.rs` | `AddBuff` |
| 3 | Damage | `skill/behavior/damage.rs` | `DealDamage` |
| 4 | Heal | `skill/behavior/heal.rs` + `buff_actions/heal.rs` | `Heal` |
| 5 | Shield | `buff_actions/shield.rs` | `Shield` |
| 6 | Dot / Burn / Poison | `buff_actions/dot.rs` | `DotTick` |
| 7 | ExPoint | `skill/behavior/ex_point.rs` | `ExChange` |

For each migration:
1. Route through `EffectEngine` instead of old registry.
2. Run battle replay regression — assert output matches.
3. Delete old handler.

### Phase 4: Migrate Trigger/Passive System

- Replace `trigger/combat.rs` scan loop → `EffectEngine::emit(BattleEvent::PostSkill { ... })`.
- Replace `passives/executor.rs` battle-start loop → `EffectEngine::emit(BattleEvent::BattleStart { ... })`.
- Replace `passives/ally_be_attacked.rs` → `EffectEngine::emit(BattleEvent::BeAttacked { ... })`.
- Delete `TriggerPass` trait — its passes (HP sync, blood pool sync, EX sync) become built-in post-emit hooks in `EffectEngine`.

### Phase 5: Exotic Handlers as Custom

Register hand-coded handlers implementing `CustomActionHandler`:

| Handler name | Current location | Mechanic |
|---|---|---|
| `nuodika_damage` | `skill/behavior/nuodika_damage.rs` | HP-scaling damage |
| `raspberry` | `buff_actions/raspberry.rs` | Sub-skill chain |
| `magic_circle` | `mechanics/magic_circle.rs` | Nested skill embedding |
| `bloodtithe` | `skill/behavior/bloodtithe.rs` | Blood pool cost/gain |
| `empathy` | `skill/behavior/empathy.rs` | Shared damage |
| `catapult` | `skill/behavior/catapult.rs` | Bounce targeting |
| `monster_change` | `skill/behavior/misc.rs` | Form transformation |

Referenced via `ActionStep::Custom { handler_name }` in config.
Delete old `BehaviorAction` and `BuffActionHandler` traits + registries.

### Phase 6: Cleanup

- Delete `buff/` module (already a pass-through).
- Collapse remaining `buff_actions/` into `effect/custom/` (only exotic handlers remain).
- Delete `skill/behavior/action.rs` registry.
- Simplify `SkillExecutor` — becomes a thin wrapper calling `EffectEngine::emit`.
- Remove `trigger/` directory (absorbed into `EffectEngine`).
- Remove `passives/` directory (absorbed into `EffectEngine`).
- Update docs.

---

## Verification Strategy

### Per-phase regression

- **Battle replay test:** Replay recorded `BeginRoundOper` sequences through both old and new paths. Assert `FightRound` output matches byte-for-byte.
- **Per-handler unit test:** Migrated handler produces identical `Vec<ActEffect>` as old handler for same inputs.

### Coexistence invariant

During phases 3-5, both old and new systems are active:
- New `EffectEngine` handles migrated effects.
- Old registries handle unmigrated effects.
- Dispatch order: `EffectEngine` first → if no match, fall through to old registry.

---

## File Structure (End State)

```
battle/
├── effect/                    # NEW — unified effect system
│   ├── mod.rs                 # EffectEngine, emit(), run_pipeline()
│   ├── types.rs               # Effect, TriggerKind, ActionStep, ConditionExpr, TargetSelector
│   ├── interpreter.rs         # interpret_action() for data-driven ActionSteps
│   ├── condition.rs           # ConditionExpr evaluation
│   ├── loader.rs              # Config tables → Vec<Effect>
│   └── custom/                # Hand-coded CustomActionHandler impls
│       ├── mod.rs
│       ├── nuodika.rs
│       ├── raspberry.rs
│       ├── magic_circle.rs
│       ├── bloodtithe.rs
│       ├── empathy.rs
│       └── catapult.rs
├── skill/                     # Simplified — executor + cache only
│   ├── executor.rs            # Thin wrapper → EffectEngine::emit
│   ├── cache.rs               # SKILL_CACHE (still needed for config lookup)
│   └── damage.rs              # calculate_damage, should_crit_hit (shared math)
├── manager/                   # Unchanged
├── context/                   # Simplified — fewer context variants
├── event_queue.rs             # Unchanged (host assembly still needed)
├── phase/                     # Unchanged (round orchestration)
└── docs/
```

---

## What This Replaces

| Old module | Lines (approx) | Replaced by |
|---|---|---|
| `buff/` | 40 | Deleted (was pass-through) |
| `buff_actions/` (25+ files) | ~3000 | `effect/interpreter.rs` + `effect/custom/` |
| `skill/behavior/` (20+ files) | ~2500 | `effect/interpreter.rs` + `effect/custom/` |
| `skill/condition/` | ~500 | `effect/condition.rs` |
| `skill/classification.rs` | ~200 | Internal optimization in `EffectEngine` |
| `passives/` (10 files) | ~1500 | `EffectEngine::emit(BattleStart/EnterFight/BeAttacked)` |
| `trigger/` (10 files) | ~1200 | `EffectEngine::emit(PostSkill)` + post-emit hooks |

**Total replaced:** ~9000 lines across 80+ files → ~2000 lines in `effect/` module.
