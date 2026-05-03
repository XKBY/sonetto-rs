#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum BehaviorType {
    Damage {
        rate: i32,
    },
    Heal {
        rate: i32,
    },
    HealByTwoAttr {
        missing_percent: i32,
        caster_hp_percent: i32,
    },
    AddBuff {
        buff_id: i32,
        count: i32,
    },
    CatapultBuff {
        primary_stacks: i32,
        duration: i32,
        buff_id: i32,
        catapult_stacks: i32,
        catapult_cap: i32,
    },
    AddTargetBuffByPoison {
        stack_count: i32,
        duration: i32,
        buff_id: i32,
        max_targets: i32,
    },
    RealDamageSelfAndAddBuffToTarget {
        amount_permille: i32,
        buff_id: i32,
    },
    /// Genesis bonus damage scaling on the caster's stored Empathy
    /// (a.k.a. injury bank). Encoded as `60038#multiplier_permille` on
    /// skill_effect rows whose primary damage already comes from
    /// `damageRate`. The bonus = `current_empathy × multiplier / 1000`
    /// and emits as an additional `OriginDamage(130)` effect alongside
    /// the primary damage. Genesis DMG ignores defense.
    OriginDamageFromInjuryBank {
        multiplier_permille: i32,
    },
    /// Kakania's EX "Id, Ego and Superego" behavior — same Genesis
    /// bonus formula as `OriginDamageFromInjuryBank` but the
    /// caster's stored Empathy is RESET to 0 immediately after the
    /// bonus is computed. Encoded as
    /// `60040#multiplier_permille[#...]` per skill 30800131
    /// (`behavior1 = 60040#10000#1#0`). The in-game ability text:
    /// "1-target attack. Deals 400% Mental DMG plus
    /// (Current [Empathy] × 1000%) Genesis DMG to the target,
    /// resets [Empathy] to zero …"
    ConsumeInjuryBankAndDamage {
        multiplier_permille: i32,
    },
    AddExPoint {
        amount: i32,
    },
    AddExPointWithMax {
        amount: i32,
    },
    LostLife {
        mode: i32,
        attr_id: i32,
        permille: i32,
        behavior_id: i32,
    },
    Bloodlust {
        amount: i32,
    },
    AverageLife,
    BloodPoolValueChange {
        amount: i32,
    },
    BloodPoolMaxChange {
        amount: i32,
    },
    AttrModify {
        attr_id: i32,
        amount: i32,
    },
    BeAttackedAssassinate {
        attr_id: i32,
        amount: i32,
    },
    ConsumeBuffByTypeId {
        type_id: i32,
        count: i32,
    },
    Disperse,
    DisperseForce {
        buff_id: i32,
    },
    Purify,
    ChangePower {
        amount: i32,
    },
    AttrFix {
        attr_id: i32,
        amount: i32,
    },
    /// Variant of `AttrFix` whose `amount` is derived from the
    /// caster's missing HP. LIVE encodes it as
    /// `60033#<step_permille>#<attr_id>#<bonus_per_stack>#<max_stacks>` —
    /// e.g. Semmelweis Insight III `308801821 slot 6 = '60033#100#205#75#8'`
    /// reads as: every 10% of MaxHP missing grants +7.5% to attr 205
    /// (AddDmg), capped at 8 stacks (60% total). Other slots that
    /// fire on the same trigger use plain `AttrFix` for the
    /// non-scaled attribute bonuses.
    AttrFixByLoseHp {
        step_permille: i32,
        attr_id: i32,
        bonus_per_stack: i32,
        max_stacks: i32,
    },
    /// Bonus `OriginDamage` emission whose value is
    /// `caster.attr[attr_id] × permille × stack_count_on_target / 1000`,
    /// where `stack_count_on_target` is the number of buff stacks on
    /// the target whose `bufftype.include_types` contains `group_id`.
    /// LIVE encodes it as `60127#<mode>#<attr_id>#<permille>#<group_id>` —
    /// e.g. Tuesday's Lock-Sound mass attack
    /// `30980131 slot 2 = '60127#1#102#300#7'` reads as: Tuesday's
    /// `ATK × 30% × number of Poison stacks (group 7) on the target`,
    /// per her in-game text "deals an additional (caster's ATK ×
    /// number of instances of [Poison] on the target × 30%) Genesis
    /// DMG". `mode` is preserved but currently unused.
    OriginDamageByAttrAndBuffGroupSize {
        mode: i32,
        attr_id: i32,
        permille: i32,
        group_id: i32,
    },
    SkillRateUp {
        rate: i32,
    },
    ConsumePowerDirectUseSkill {
        count: i32,
        skill_id: i32,
    },
    DirectUseSkill {
        skill_id: i32,
    },
    DirectUseBigSkill,
    ConsumeExPointAddAttr {
        min_consume: i32,
        max_consume: i32,
    },
    SkillRateUpBySelfBuffType {
        buff_type_id: i32,
        rate: i32,
    },
    SkillRateUpByBuffType {
        rate: i32,
        buff_types: Vec<i32>,
    },
    RandomUseSkill {
        raw: String,
    },
    /// `40006#new_monster_id#probability_permille#flag` — transforms
    /// the target entity into a different monster form. See
    /// `mechanics::phase_change` for the implementation.
    MonsterChange {
        new_monster_id: i32,
        probability_permille: i32,
    },
    Kill,
    Summon {
        skill_id: i32,
    },
    RaspberryAddCount {
        attr_id: i32,
        rate: i32,
    },
    ConsumeBloodAddBuff {
        consume: i32,
        buff_id: i32,
        count: i32,
    },
    ConsumeBloodAddBuff2 {
        consume: i32,
        buff_id: i32,
        count: i32,
    },
    AddBuffRanId {
        pool_buff_id: i32,
        count: i32,
    },

    AddMagicCircle {
        circle_id: i32,
    },

    /// Applies attribute modifiers to entities on configured sides
    /// (typically while a magic circle / array skill is active).
    ///
    /// Encoded as `60076#side#attr#permille[#side2#attr2#permille2]...`
    /// where `side` is 1 (caster's team) or 2 (opposing team).
    /// Each `(side, attr_id, permille)` tuple registers an attr
    /// bonus on every entity on that side and emits one `Attr(26)`
    /// marker per entity.
    MagicCircleAttr {
        modifiers: Vec<(i32, i32, i32)>,
    },

    DirectUseGroupAndStarSkill {
        group: i32,
        rank: i32,
    },

    ReplaceBuff2 {
        source_buff_ids: Vec<i32>,
        replacement_buff_id: i32,
        duration: i32,
        count: i32,
    },

    /// TODO: implement when shell/summon system is built
    ShellUseSkill {
        group: i32,
        skill_id: i32,
    },
    /// TODO: assigns a skill to a shell entity slot (combat only)
    ShellAssign {
        slot: i32,
        skill_id: i32,
    },
    /// Like Purify but removes specific buff type ids (combat only)
    PurifyX {
        type_ids: Vec<i32>,
    },

    /// TODO: implement crystal card deck manipulation
    CrystalAddCard,

    IgnoreSkillConfigDamageRate,

    LostAllLifeByAttr {
        caster_attr: i32,
        caster_amount: i32,
        target_attr: i32,
        target_amount: i32,
    },

    DamageRealLostLife {
        buff_id: i32,
        duration: i32,
        rate: i32,
    },

    NuoDiKaDamage {
        primary_buff_id: i32,
        primary_rate: i32,
        secondary_buff_id: i32,
        secondary_rate: i32,
        self_loss_param: i32,
    },

    /// Round-start poison-settle on the carrier (the entity holding the
    /// passive `30980151` slot whose `behavior1=60073#1`). Walks the
    /// carrier's own Poison / DeadlyPoison buffs, emits one
    /// `OriginDamage(130)` per stack at `caster.atk × permille / 1000`,
    /// and decrements `duringTime` by `rounds` UNLESS the carrier also
    /// holds a `LockPoison(810)` buff from the array owner — Tuesday's
    /// `30980131` "lock duration" debuff applied via
    /// `magic_circle 22100003.enemy_buff`. Per the in-game text:
    /// "At the start of the round, resolve 1 round of [Poison] effects."
    /// Combined with Tuesday's array Lock-effect: "if tick is 2 after 3
    /// rounds it still be 2 not 0". The 810 lock pins `duringTime` so
    /// the same Poison stacks keep ticking damage every round.
    SettleDotAndCostDotDuration {
        rounds: i32,
    },

    Unknown {
        raw: String,
    },
}
