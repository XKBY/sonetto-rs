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
    RealDamageSelfAndAddBuffToTarget {
        amount_permille: i32,
        buff_id: i32,
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
    MonsterChange,
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

    Unknown {
        raw: String,
    },
}
