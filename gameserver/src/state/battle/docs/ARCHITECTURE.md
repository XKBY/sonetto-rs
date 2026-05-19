# Battle System Architecture

This document maps the current module structure of `gameserver/src/state/battle/`,
identifies architectural friction points, and proposes deepening opportunities.

Vocabulary follows the "deep module" framework:
- **Module** — anything with an interface and an implementation.
- **Interface** — everything a caller must know (types, invariants, ordering, config).
- **Depth** — leverage at the interface: lots of behaviour behind a small interface.
- **Seam** — where an interface lives; a place behaviour can be altered without editing in place.
- **Locality** — change/bugs/knowledge concentrated in one place.
- **Deletion test** — imagine deleting the module. If complexity vanishes, it was a pass-through.

---

## Module Map

```
battle/
├── skill/                  # Skill execution engine
│   ├── executor.rs         # SkillExecutor — core orchestrator + god-struct
│   ├── behavior/           # 20+ behavior clusters (damage, heal, add_buff, ...)
│   │   ├── action.rs       # BehaviorAction trait + BEHAVIOR_REGISTRY
│   │   └── *.rs            # One file per behavior cluster
│   ├── cache.rs            # SKILL_CACHE: skill_effect_id → Vec<ParsedBehavior>
│   ├── condition/          # Condition parsing + evaluation
│   ├── classification.rs   # Pre-filter heuristics (has_combat_reactive_condition, etc.)
│   ├── damage.rs           # calculate_damage, should_crit_hit
│   ├── euphoria.rs         # Skill ID remapping under euphoria state
│   ├── targets.rs          # TargetResolver, alive_enemies, get_entity
│   ├── phase.rs            # PhaseFilter, TriggerState
│   └── sibling_coalesce.rs # Merging sibling skill effects
│
├── buff/                   # SHALLOW — 2 functions forwarding to buff_actions
│   ├── mod.rs              # re-exports apply.rs
│   └── apply.rs            # apply_buff_effects, pre_buff_effects (38 lines)
│
├── buff_actions/           # The REAL buff feature execution engine
│   ├── mod.rs              # Dispatch logic, feature_spec, for_each_buff_feature
│   ├── action.rs           # BuffActionHandler trait, BUFF_HANDLER_REGISTRY
│   ├── dispatcher.rs       # dispatch_stage, dedupe_dead_effects_against_prior_steps
│   ├── result.rs           # ActionResult type
│   └── *.rs                # 25+ handler files (attr, dot, heal, shield, ...)
│
├── passives/               # Battle-start + enter-fight passive execution
│   ├── executor.rs         # run_battle_start — 20-variant Pass enum
│   ├── collector.rs        # Collects passive skill IDs from entities
│   ├── ally_be_attacked.rs # Inline BeAttacked reactive expansion
│   ├── inject.rs           # Passive injection helpers
│   └── steps/              # Step builders (skill, passive, cards, temp_card)
│
├── trigger/                # Mid-combat reactive trigger system
│   ├── combat.rs           # TriggerEvent, skill_should_fire, run_combat_triggers
│   └── passes/             # Post-fire side-effect passes (HP sync, blood pool, etc.)
│       ├── mod.rs           # TriggerPass trait
│       └── *.rs             # 7 pass implementations
│
├── manager/                # State managers
│   ├── buff_mgr.rs         # BuffMgr — BuffInstance storage, add/remove/query
│   ├── ex_point_mgr.rs     # Moxie/Faith tracking
│   ├── round_mgr.rs        # Round state, action ordering
│   ├── card_mgr.rs         # Card/deck management
│   ├── fight_data_mgr.rs   # Managers bundle struct
│   └── *.rs                # wave_mgr, summon_mgr, etc.
│
├── mechanics/              # Cross-cutting game mechanics
│   ├── bloodtithe.rs       # Bloodtithe state machine
│   ├── empathy.rs          # Empathy buff detection
│   └── ...
│
├── context/                # Context bundles threaded through execution
│   ├── effect_context.rs   # EffectContext (fight + managers + mechanics refs)
│   ├── behavior_context.rs # BehaviorContext (fight snapshot for condition eval)
│   └── fight_context.rs    # FightContext (mutable fight + managers + mechanics)
│
├── event_queue.rs          # EventQueue, BattleEvent, drain_to_fight_steps
├── entity/                 # Entity queries and state
├── fight/                  # Fight struct helpers, defender logic
├── round/                  # Round phases, passive_phase
├── phase/                  # Phase orchestration (build_round_output, round_open)
├── card/                   # Card/deck system
└── types/                  # Shared type definitions (BehaviorType, ConditionType, etc.)
```

---

## Dependency Flow

```
                    ┌─────────────┐
                    │  phase/     │  (orchestrates rounds)
                    └──────┬──────┘
                           │ calls
              ┌────────────┼────────────┐
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │passives/ │ │trigger/  │ │ round/   │
        │executor  │ │combat    │ │phases    │
        └────┬─────┘ └────┬─────┘ └────┬─────┘
             │             │             │
             └──────┬──────┘             │
                    ▼                    │
           ┌────────────────┐           │
           │ skill/executor │◄──────────┘
           │ (SkillExecutor)│
           └───────┬────────┘
                   │ dispatches to
          ┌────────┼────────┐
          ▼        ▼        ▼
    ┌──────────┐ ┌────┐ ┌──────────────┐
    │behavior/ │ │buff│ │buff_actions/ │
    │clusters  │ │    │ │handlers      │
    └──────────┘ └─┬──┘ └──────────────┘
                   │ forwards to
                   ▼
            ┌──────────────┐
            │buff_actions/ │  (same module)
            └──────────────┘
```

Key asymmetries:
- `skill/` does NOT import from `passives/` or `trigger/`.
- `passives/` and `trigger/` both call into `skill/`.
- `buff_actions/` receives `&mut SkillExecutor` so handlers can queue sub-skill fires.
- `buff/` is a dead pass-through to `buff_actions/`.

---

## Friction Points

### 1. `buff/` is a dead pass-through

**Deletion test:** Delete `buff/`. Move its 2 functions into `buff_actions/mod.rs`. Zero complexity reappears elsewhere — it was pure forwarding.

**Current state:** `buff/apply.rs` (38 lines) constructs an `EffectContext` and calls `buff_actions::apply_after_buff_add_features` or `buff_actions::apply_before_buff_add_features`. That's it.

**Impact:** Callers import from `buff::apply_buff_effects` when they could import from `buff_actions` directly. The extra module creates a false sense of encapsulation — the real interface is `buff_actions/mod.rs`.

---

### 2. `buff_actions/` has no coherent public interface

**Problem:** 20+ modules are `pub mod`, meaning any caller can reach into handler internals:
- `passives/executor.rs` imports `buff_actions::blood_pool_ex::buff_get_blood_pool_ex_point_params`
- `passives/executor.rs` imports `buff_actions::raspberry::buff_get_raspberry_params`
- `trigger/combat.rs` imports `buff_actions::blood_pool_ex::build_blood_pool_gain_ex_point_step`

These are implementation details of specific buff handlers being used as ad-hoc query APIs by unrelated modules. The seam is leaking.

**Desired state:** `buff_actions` exposes:
1. `apply_before_buff_add_features(ctx, executor, buff_id, condition_id) → Vec<ActEffect>`
2. `apply_after_buff_add_features(ctx, executor, buff_id, has_bloodpool) → Vec<ActEffect>`
3. `dispatch_stage(stage, ctx) → Vec<ActEffect>`
4. A small set of explicitly designed query helpers (blood pool params, raspberry params) — but behind a curated `pub` facade, not raw module access.

---

### 3. `skill/behavior/` uses a weaker dispatch pattern than `buff_actions/`

**Comparison:**

| Aspect | `buff_actions` (newer) | `skill/behavior` (older) |
|--------|----------------------|------------------------|
| Trait | `BuffActionHandler` | `BehaviorAction` |
| Phases | `matches` → `parse` → `execute` → `steps` | Single `execute` method |
| Matching | Explicit `matches(&self, act_type, stage) -> bool` | Internal pattern match |
| Testability | Can test `parse` and `steps` independently | Must construct full `ActionCtx` |
| Self-describing | Yes — `matches` declares ownership | No — hidden inside `execute` body |

The `BehaviorAction` trait forces each cluster to be a grab-bag: parse params, evaluate conditions, produce effects — all in one `execute` body. This makes behaviors hard to understand in isolation and impossible to unit-test without the full context bundle.

**Migration path:** Introduce `BehaviorHandler` (analogous to `BuffActionHandler`) with `matches` → `parse` → `execute` → `steps`. Migrate behaviors one at a time. Keep `BEHAVIOR_REGISTRY` as fallback for unmigrated clusters.

---

### 4. `passives/` and `trigger/` implement one concept in three places

The concept is: "given a triggering event, find skills with matching conditions, fire them, collect results."

**Three implementations:**

| Location | When | Shape |
|----------|------|-------|
| `passives/executor.rs` | Battle start, enter fight | 20-variant `Pass` enum, iterates entities, calls `execute_skill` |
| `trigger/combat.rs` | Mid-combat (after each action) | `TriggerEvent` struct, `TriggerPass` trait, calls `skill_should_fire` + `execute_skill` |
| `passives/ally_be_attacked.rs` | Inline reactive (enemy skill) | Scans for `BeAttacked` conditions, fires inline into enemy step |

All three share:
- `EventQueue` → `drain_to_fight_steps` plumbing
- Condition evaluation via `skill/condition/`
- Skill firing via `SkillExecutor`
- Step wrapping via `fight_step/`

But each has its own calling convention, entity iteration logic, and condition pre-filtering.

**Unified seam:** `fire_reactives(trigger_event, entity_set, phase_filter, ctx) → Vec<FightStep>`. The three call sites become adapters that construct the appropriate inputs.

---

### 5. `SkillExecutor` is a god-struct (12+ fields, 4 unrelated concerns)

**Current fields grouped by concern:**

```rust
// Effect accumulation
pub side_effects: Vec<ActEffect>
pub pending_monitor_triggers: Vec<(i64, i32)>
pub pending_buff_dels: Vec<(i64, i32)>
pub pending_summons: Vec<PendingSummon>
pub pending_monster_changes: Vec<PendingMonsterChange>

// Damage modifiers (per-execution)
pub pending_target_rate_bonus: HashMap<i64, i32>
pub pending_global_rate_bonus: i32
pub pending_attr_bonus: HashMap<(i64, i32), i32>
pub pending_bloodtithe_preview: HashMap<i32, (i32, i32)>

// Recursion control
call_depth: usize
override_damage_targets: Option<Vec<i64>>

// Skill context
current_skill_context: Option<(i32, i64)>
```

Every handler receives `&mut SkillExecutor` and can mutate any field. A buff handler that only needs to queue an effect can accidentally touch damage modifiers. There's no interface narrowing.

**Split candidates:**
- `EffectSink` — collects effects + deferred mutations
- `DamageModifiers` — rate bonuses, attr bonuses, bloodtithe preview
- `RecursionGuard` — depth tracking, reentry prevention, context stack

---

### 6. `skill/condition/` is deep but leaks pre-filter logic to callers

**Current caller burden** (from `trigger/combat.rs` imports):
```rust
use skill::classification::{
    has_combat_reactive_condition,
    has_injury_reactive_condition,
    has_be_attacked_reactive_condition,
    CombatPassiveScanMode,
};
use skill::condition::parser::parse_condition;
use skill::condition::buff::deleted_matches;
use skill::condition::scope::is_round_start_only;
```

Six imports to answer one question: "does this skill fire for this event?"

The `classification.rs` functions exist because full condition evaluation is expensive — they're cheap pre-filters that pattern-match `ConditionType` variants. But they duplicate condition knowledge: when a new condition type is added, both the evaluator AND the classifier must be updated.

**Desired interface:** `should_fire(skill_id, trigger_state, phase) -> bool` — internally does the cheap pre-filter, then full eval if needed. Callers never see `ConditionType` variants.

---

## Proposed Refactoring Order

Priority is based on: (a) how much friction the current state causes when adding features, (b) how isolated the change is (low blast radius first).

1. **Collapse `buff/` into `buff_actions/`** — trivial, zero-risk, removes confusion.
2. **Narrow `buff_actions/` public surface** — make handler modules `pub(super)`, export query helpers explicitly.
3. **Extract `should_fire` from condition/classification** — reduces caller coupling, enables testing the question directly.
4. **Unify reactive firing** — biggest payoff but highest risk; do after (3) stabilizes the condition interface.
5. **Split `SkillExecutor`** — do after (4) since the unified reactive module will clarify what each subsystem actually needs from the executor.
6. **Migrate `BehaviorAction` → `BehaviorHandler`** — incremental, one behavior at a time, lowest urgency since the current pattern works (just isn't testable).

---

## Dispatch Pattern Reference

Both registries use "first match wins" with an object-safe trait:

```rust
// buff_actions (newer, preferred)
trait BuffActionHandler {
    type Params;
    fn matches(&self, act_type: &str, stage: BuffStage) -> bool;
    fn parse(&self, parts: &[&str], ctx: &BuffActCtx) -> Self::Params;
    fn execute(&self, params: &mut Self::Params, ctx: &mut BuffActCtx) {}
    fn steps(&self, params: Self::Params, ctx: &BuffActCtx) -> ActionResult;
}

// skill/behavior (older, monolithic)
trait BehaviorAction {
    fn execute(
        &self,
        behavior: &BehaviorType,
        ctx: &mut ActionCtx,
        condition: &ConditionType,
    ) -> Option<Result<Vec<ActEffect>>>;
}
```

The `BuffActionHandler` pattern is strictly superior for:
- Self-documentation (what does this handler claim?)
- Testability (parse and steps are pure)
- Incremental migration (old and new registries coexist)

---

## Context Bundle Inventory

Three context structs exist, each threading slightly different state:

| Struct | Location | Contains | Used by |
|--------|----------|----------|---------|
| `FightContext` | `context/fight_context.rs` | `&mut Fight`, `&mut Managers`, `&mut Mechanics` | `passives/`, `trigger/`, `phase/` |
| `EffectContext` | `context/effect_context.rs` | `&Fight`, `&mut Managers`, `&mut Mechanics`, `caster_uid`, `target` | `buff_actions/` |
| `BehaviorContext` | `context/behavior_context.rs` | `&Fight` (snapshot), `caster_uid`, `target_uid`, `skill_id` | `skill/behavior/` |

`FightContext` owns mutable `Fight`; `EffectContext` borrows immutable `Fight`; `BehaviorContext` holds a cloned snapshot. This three-tier split exists because:
- Buff features must not mutate `Fight` directly (effects are applied later by the caller).
- Behavior condition evaluation needs a stable snapshot (mutations during iteration would invalidate conditions).
- Phase orchestration needs full mutability.

This is a load-bearing design decision, not accidental complexity.
