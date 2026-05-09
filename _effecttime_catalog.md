# `effectTime` semantic catalog

Source: `data/excel2json/buff_act.json`. 434 total buff_acts across 42
distinct `effectTime` values.

This document maps every `effectTime` value in the config to its
inferred phase semantics, the buff_acts that use it, and (where
relevant) the currently-hardcoded engine path that an `effectTime`-aware
dispatcher would retire.

The thesis is simple: **the engine currently mostly ignores `effectTime`,
dispatching by hardcoded buff-id / skill-id checks instead. Honoring
`effectTime` as the dispatch axis would retire most of the hero-specific
hardcoding without authoring a single overlay JSON.**

Cross-reference rule used below: an `effectTime` value's semantic phase
is inferred from the buff_acts that share it AND from prydwen
descriptions of carrier buffs that exercise it. Where the
inference is conjectural rather than fixture-confirmed, that's flagged.

## Summary table

| effectTime | Phase | Buff_act count | Carrier buff count | Engine status | Highest-leverage retire |
|--:|--|--:|--:|--|--|
| -1 | special (cycle transfer) | 1 | – | – | – |
| **0** | **on-add (default)** | **217** | **bulk** | **handled** | n/a (default path) |
| 12 | on-death / revive | 4 | small | mostly-handled | – |
| 101 | round-start: DOT settle | 2 | small | partial | tighten settle ordering |
| **102** | **round-start: cure / refresh / cast-channel** | **10** | **~216** | **partial** | **`mechanics/advanced_cure.rs` hardcoded** |
| 103 | round-start: target add / raspberry | 7 | small | partial | – |
| 104 | round-start: dot variant + channel | 8 | small | partial | – |
| 105 | round-start: card grant | 14 | medium | partial | per-hero card-grant hardcodes |
| 106 | round-start: card transform | 3 | small | partial | – |
| 201 | combat: target add / equip | 10 | medium | partial | – |
| 202 | combat: attr / be-attacked-flag | 7 | medium | partial | – |
| 203 | combat: attr-when-attack | 18 | medium | partial | – |
| 204 | one-off attr-by-layer | 1 | small | – | – |
| **207** | **be-attacked: defensive** | **6** | **~462** | **partial** | shield/dodge generic |
| **208** | **on-cast (after carrier uses skill)** | **11** | **~117** | **mostly-hardcoded** | **Sotheby holder, Bullet, ConsumeBuffAddExPoint** |
| **209** | **be-attacked: reactive (rebound / share-hurt)** | **11** | **~94** | **partial** | rebound generic |
| 210 | post-attack: counter / cure | 10 | medium | partial | – |
| 211 | on-kill / lethal | 5 | small | partial | – |
| 212 | post-skill (entire-skill resolution) | 11 | medium | mostly-hardcoded | Tuesday `30980142→30980145`, Pickles Hedonism |
| 213 | shell tag | 1 | small | – | – |
| 301 | round-start: ex-point + channel | 4 | small | partial | – |
| **302** | **round-end: DOT cycle** | **17** | **~277** | **mostly-hardcoded** | **`mechanics/dot.rs` hardcoded carrier list** |
| 303 | round-end: cure-end / del-by-type | 4 | small | partial | – |
| 304 | round-end: channel close | 5 | small | partial | – |
| **305** | **overflow-handler (ExPointOverflowBank)** | **2** | **small** | **mostly-hardcoded** | **`mechanics/shadowcloak.rs` Rubuska** |
| 306 | round-end: card add | 2 | small | partial | – |
| **307** | **round-end: injury-bank / protect / monitor** | **6** | **medium** | **mostly-hardcoded** | **`mechanics/empathy.rs` Kakania InjuryLogback** |
| 401 | beat-back family | 4 | small | partial | – |
| 402 | layer-overflow trigger | 1 | small | – | – |
| 900 | emitter-energy add | 1 | small | – | – |
| 901 | emitter-rend / balance | 2 | small | – | – |
| 903 | crit-rate alter v2 | 1 | small | – | – |
| 908 | buff-entity v3 | 2 | small | – | – |
| 1041 | cast-channel + count | 2 | small | – | – |
| 1051 | dream / battle-selection | 2 | small | – | – |
| 1061 | record-by-round | 2 | small | – | – |
| 1062 | add-card-cast-channel | 1 | small | – | – |
| 2061 | hurt-extra | 1 | small | – | – |
| 2081 | extended on-cast variants | 6 | small | – | – |
| 2091 | extended on-attack variants | 9 | small | – | – |
| 2101 | post-act regain power | 2 | small | – | – |
| 2111 | end-of-everything cure | 1 | small | – | – |

The four bolded rows (102, 208, 302, 307) plus 305 are where the
hardcoding density is highest. Honoring `effectTime` for these five
slots would retire most of the named "shadow subsystem" mechanics
file content.

---

## Phase 0 — `effectTime=0` (on-add, default)

217 buff_acts. The default. Buff_act fires once when the buff is
applied to its target. This is what `BuffStage::AfterBuffAdd` already
implements correctly. Includes:

- `100..115` — all the static `Attr*` flat / multiplier modifiers
- `301..304` — taunt, dizzy, petrified, sleep, frozen status flags
- `401..407` — control / disable status flags

No change needed. This is the dispatcher's default branch when the
config doesn't specify a non-zero `effectTime`.

---

## Phase 102 — round-start: cure / refresh / cast-channel

10 buff_acts, ~216 carrier buffs.

| act_id | type | what it does |
|--:|--|--|
| 201 | Cure | round-start heal (bulk: 68 carriers) |
| 508 | ExPointAdd | round-start moxie/faith add |
| 701 | CardLevelAdd | round-start level bump |
| 733 | ContinueChannel | round-start channel tick |
| 742 | CastChannel | round-start cast |
| 798 | RandomCardLevelChange | round-start randomize |
| **849** | **AdvancedCure** | **Sotheby's `30091111` Cure HoT — already in `mechanics/advanced_cure.rs`** |
| 912 | CountContinueChannel | round-start count tick |
| 1045 | ChangeCardToSelf | round-start card swap |
| 1059 | FictionHp | round-start "fake HP" |

**Currently hardcoded**: `mechanics/advanced_cure.rs` walks every
carrier of buff feature `849` at round start. Generic but
mechanic-scoped. An `effectTime=102` dispatcher would absorb both
`AdvancedCure` and `Cure 201` into the same round-start cure pass,
and the cast-channel siblings into the round-start channel pass.

**Validation against in-game text**:
- Sotheby Cure (prydwen): *"When a round starts, restores HP based on the caster's ATK"* ← matches `effectTime=102`.

---

## Phase 207 — be-attacked: defensive

6 buff_acts, ~462 carrier buffs. Massive surface area dominated by
Shield (`501` has 405 carriers).

| act_id | type | what it does |
|--:|--|--|
| 304 | ExPointFix | be-attacked moxie shift |
| **501** | **Shield** | **HP shield absorber (405 carriers)** |
| 505/507 | DodgeSpecSkill / 2 | dodge specific incoming skills |
| 510 | DamageNotMoreThan | damage cap |
| 782 | ShieldByGougeCoin | conditional shield |

**Currently handled**: `buff_actions/shield.rs` and adjacent files
already implement this generically. `effectTime=207` is mostly
already honored in spirit.

---

## Phase 208 — on-cast (THIS is the Sotheby slot)

**11 buff_acts, ~117 carrier buffs.** The dispatcher gap that
`_30091120_design.md` is patching the symptom of.

| act_id | type | hardcoded path today | data-driven would unlock |
|--:|--|--|--|
| **205** | **Dot** | – | on-cast DOT application (e.g. on-attack-poison) |
| 503 | AddToTarget | per-hero add-to-target hooks | generic on-cast target add |
| 504 | DamageExtra | – | on-cast extra damage rider |
| 799 | ConsumeBuffAddExPoint | partial in `bloodtithe.rs` | generic consume-on-cast |
| 804 | DisperseByTag | – | generic on-cast cleanse |
| **827** | **Bullet** | partial in `mechanics/bullet.rs` | generic on-cast bullet emission |
| **850** | **AddBuffBoth** | **`add_buff_both.rs:78` skips `30091120` + `executor.rs:743` matches `30090111\|30090112`** | **Sotheby holder consume becomes generic** |
| 856 | AddToBuffEntity2 | – | generic on-cast buff-to-entity |
| **874** | **DiamondBullet** | partial bullet code | generic |
| 1051 | CrystalAddBuff | – | generic |
| 1082 | UseSkillConsumeFromAddEmitterEnergy | – | generic |

**Currently hardcoded around**: this entire phase. Sotheby's holder
is the canary; Bullet (827) is partially handled but inconsistent;
the others fire on different code paths or not at all.

**Validation against in-game text**:
- Sotheby basic: *"Mass attack. ... Inflicts [Poison] for 5 rounds on the targets hit"* ← "on the targets hit" = on-cast trigger ⇒ `effectTime=208`. Matches `850 AddBuffBoth`.
- Sentinel Dread Bullet (memory note): *"Trigger Bullet"* keyword ← matches `827 Bullet effectTime=208`.

**The implementation seam**: `executor.rs` post-damage section
already runs `consume_attr_only_damage_buffs` adjacent to where
208-dispatch should happen. The cascade-trap (re-entering
`buff_feature_reactives.rs` on synthetic emissions) is the design
work — solved with a `SyntheticEmission` context tag, not by
hardcoding.

---

## Phase 209 — be-attacked: reactive

11 buff_acts, ~94 carrier buffs.

| act_id | type | what it does |
|--:|--|--|
| 303 | Rebound | reflect damage |
| 308 | BeatBack | knock-back attack |
| 743 | ReboundBasedOnDamage | scaled reflect |
| 808 | AttrByLayer | layer-based attr |
| 872 | ShareHurt | distribute damage |
| 873 | ShellLock | shell carrier |
| 900 | RandomBuffByCasterDamage | RNG buff on hit |
| 926 | ExPointAddByHit | moxie on hit |
| 1037 | BeAttackAccrualFixAttr | accrued attr |
| 1038 | BeAttackAddBuff | add buff on hit |
| 10010 | BeAttackedFromUseSkill | hit-from-skill flag |

**Currently handled**: rebound and beat-back generically. Some are
partial. `effectTime=209` dispatch would unify "I got hit, now do X."

---

## Phase 212 — post-skill (entire-skill resolution)

11 buff_acts, medium carrier count.

| act_id | type | hardcoded path today |
|--:|--|--|
| **809** | **ProbabilityAddBuff** | **`buff_feature_reactives.rs::run_probability_add_buff_reactives`** (Tuesday `30980142→30980145`) |
| 859 | BurnOverflowAddBuff | partial |
| 897 | RedOrBlueCount | – |
| 902 | ProbabilityAddBuffToSelf | – |
| 904 | AddBuffToSelf | – |
| 910 | RecordSkill | – |
| 921 | AttackConsumePowerAddBuff | – |
| 927 | AddBuffByOtherExSkill | – |
| 1050 | HeatScaleUseSkill | – |
| 1074 | UseSkillByTarget | – |
| 10008 | CopyBuffGroupByKill | – |

This is `effectTime=212` ≈ "after the entire skill (parent + all
children) finishes resolving." Pickles Hedonism Implement and
Tuesday's halo reactive both live here. Currently the
`buff_feature_reactives.rs` pass walks for `ProbabilityAddBuff`
specifically. Generalizing to all `effectTime=212` acts via the
same pass would let the rest of these mechanics participate without
adding new files.

---

## Phase 302 — round-end: DOT cycle

**17 buff_acts, ~277 carrier buffs.** The biggest single hardcoding
target after phase 208.

| act_id | type | hardcoded path |
|--:|--|--|
| 202, 211 | Dot variants | – |
| 605 | ExPointDel | – |
| **726** | **Burn** | **partial** |
| 732 | ContinueChannel | – |
| 759 | UseSkillToEnemy | – |
| **803** | **Poison** | **`mechanics/dot.rs` hardcoded carrier list (112 carriers!)** |
| 825 | ConsumeBuffContinueChannel | – |
| **844** | **DeadlyPoison** | **`mechanics/dot.rs`** |
| 862 | PaperCircleContinueChannel | – |
| 906, 948, 954 | various | – |
| 1031 | ConsumeBuffAddBuffContinueChannel | – |
| 1035 | Seed | – |

**Validation against in-game text**:
- Sotheby Poison keyword: *"At the end of a round, takes Genesis DMG based on the caster's ATK. Can stack."* ← matches `effectTime=302 + Poison 803`.
- Tuesday DeadlyPoison: same end-of-round phase, different damage tier.

`mechanics/dot.rs` already implements this generically by walking
buffs whose features include `803` or `844`. An `effectTime=302`
dispatcher would absorb that AND pick up `Burn 726`,
`ConsumeBuffContinueChannel 825`, `ColdSaturdayHurt 948`, and `Seed
1035` for free.

---

## Phase 305 — overflow-handler (ExPointOverflowBank)

2 buff_acts. Small but high-leverage.

| act_id | type | hardcoded path |
|--:|--|--|
| **806** | **ExPointOverflowBank** | **`mechanics/shadowcloak.rs` Rubuska-specific** |
| 1020 | UseBloodPoolCount | partial |

`ExPointOverflowBank 806` carriers include `30620132` (Rubuska's
Shadow Cloak), `22100003`, etc. The mechanic is "when ex-point
gain would exceed cap, divert overflow to bank" — currently
implemented per-hero. `effectTime=305` would generalize.

---

## Phase 307 — round-end: injury-bank / protect / monitor

6 buff_acts.

| act_id | type | hardcoded path |
|--:|--|--|
| **768** | **InjuryLogback** | **`mechanics/empathy.rs` Kakania-specific** |
| 899 | ProtectTargetUseSkill | – |
| 1019 | LostHpCountAddBuff | – |
| 1024 | MonitorContinueChannel | – |
| 1027 | AddBuffByChargingTimes | – |
| 1061 | AddToBuffEntity | – |

Kakania's `30800121/30800122` carry `768 InjuryLogback`. Currently
`mechanics/empathy.rs` walks specifically for those buff_ids.
`effectTime=307` dispatch would absorb both AND pick up the
protect/monitor variants.

---

## Implementation roadmap (NOT yet a brief)

The dispatcher refactor is multi-phase. Sketching the order:

### Phase A: catalog + tracing (this doc + a probe)

Add a `tracing::trace!` line per buff_act dispatch with `effectTime`
+ `act_type` + `buff_id`. Run battles with
`RUST_LOG=buff_act_dispatch=trace`. Confirm which `effectTime`
values are observed in fixtures vs theoretically present in
`buff_act.json`. Numbers in the table above are theoretical; the
trace tells us which slots actually fire.

### Phase B: `BuffStage` extension

Currently `BuffStage` has `AfterBuffAdd` (and similar). Extend:

```
enum BuffStage {
    AfterBuffAdd,            // effectTime == 0
    RoundStart_Cure,         // 102
    RoundStart_Card,         // 105
    BeAttacked_Defensive,    // 207
    OnCast,                  // 208
    BeAttacked_Reactive,     // 209
    PostAttack,              // 210
    OnKill,                  // 211
    PostSkill,               // 212
    RoundEnd_DOT,            // 302
    OverflowHandler,         // 305
    RoundEnd_InjuryBank,     // 307
    ...
}
```

Existing handlers gain a `BuffStage` filter so they only fire in
the right phase. Most existing handlers' `matches()` predicate
should narrow to one stage.

### Phase C: dispatch sites

Each stage gets one explicit dispatch site:

- **Round-start cure** (`102`) — round_mgr.rs at the round-open
  position where `mechanics::advanced_cure::run_round_start` lives
  today
- **On-cast** (`208`) — executor.rs post-damage, adjacent to
  `consume_attr_only_damage_buffs`
- **Be-attacked-defensive** (`207`) — already inside
  `damage.rs::execute_skill_damage` shield evaluation
- **Be-attacked-reactive** (`209`) — `trigger/passes/be_attacked.rs`
  generalizes
- **Post-skill** (`212`) — `trigger/passes/buff_feature_reactives.rs`
  generalizes (currently specific to `ProbabilityAddBuff`)
- **Round-end DOT** (`302`) — `mechanics/dot.rs::run_round_end`
  generalizes
- **Overflow-handler** (`305`) — `mechanics/shadowcloak.rs::on_ex_overflow`
  generalizes
- **Round-end injury-bank** (`307`) — round_mgr.rs round-end position

Each site iterates all carrier buffs whose features include a
buff_act with the matching `effectTime`.

### Phase D: cascade protection

Add `SyntheticEmission` context tag on every emission produced by
a stage dispatcher. Stages MUST honor the tag and skip dispatch
when set. Without this, on-cast emission of synthetic Poison +
Cure children would re-enter on-cast dispatch, infinite loop.

This is the single design hazard that has bitten every prior
generalization attempt (memory: "Cascade source A: BuffMgr
holder representation… Do not generalize this to all include-type-10
buffs. The user-provided failure history already falsified the
generic-holder hook").

### Phase E: hardcoding removal, file by file

Per-mechanic file removal once the corresponding stage dispatcher
covers its workload. Likely order, biggest-to-smallest:

1. `mechanics/dot.rs` (302) — biggest carrier count
2. `mechanics/advanced_cure.rs` (102)
3. `mechanics/empathy.rs` `InjuryLogback` part (307)
4. `mechanics/shadowcloak.rs` (305)
5. `add_buff_both.rs:78` skip + `executor.rs:743` match (208)
6. Tuesday halo reactive (212)
7. Pickles Hedonism (212) — nuanced (cross-stage interaction)

Each removal is a separate session under the standard byte-stable
acceptance gate.

---

## What this would NOT solve

- Effect-shape-only repairs (`AttachmentResolver`, `coalesce_late_tail_*`,
  Pickles end-of-round emission helpers) — these are LIVE serialization
  shape fixups that operate AFTER all emission is done. They're
  orthogonal to `effectTime` dispatch and need EventQueue Phase 5.
- LIVE-only conditions not in any config (the `ActOrder`,
  `UseSkillEffectTag`, `UseSpecificSkill` family for damage-passive
  selection in battle3) — those are skill_behavior_condition opcodes
  the parser doesn't recognize, separate from buff-feature `effectTime`.
- Per-character RNG selection (Recoleta target picking, AI replay
  alignment, etc.) — already documented elsewhere as upstream-of-emission.

---

## Validation prompt for the implementation session

Before writing any dispatch code, confirm with a tracing probe:

```rust
// In buff_action::dispatch
tracing::trace!(target: "buff_act_dispatch",
    buff_id, act_id, act_type, effect_time,
    stage = ?current_stage,
    skip = is_synthetic_context,
);
```

Run all three battles with `RUST_LOG=buff_act_dispatch=trace`,
group by `effect_time` × `stage`, and verify:

1. Every observed `effect_time` matches the table above
2. No `effect_time` value fires in two different stages
3. Synthetic-context emissions are correctly skipped

Only then write the staged dispatcher.
