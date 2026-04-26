#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum ConditionType {
    None,
    CombatNone, // should only trigger after entity takes a action
    EnterFight { condition_id: i32 },
    EnterFightAnd(Vec<ConditionType>),
    EnterFightOr(Vec<ConditionType>),
    HasBuffId { buff_ids: Vec<i32> },
    NoBuffId { buff_ids: Vec<i32> },
    BuffIdDel { buff_ids: Vec<i32> },

    HasTypeIdBuffMoreThan { type_id: i32, min_count: i32 },
    TypeIdBuffCountMoreThan { type_id: i32, max_count: i32 },
    TypeIdBuffCountLessThan { type_id: i32, max_count: i32 },
    HasTypeIdBuffEqual { type_id: i32, max_count: i32 },

    /// Target has at least one buff whose `bufftype.includeTypes`
    /// (`#`-delimited list) contains the matching `group` token.
    /// Encoded as `77208#group` in skill_effect rows. Tuesday's
    /// `In Mother's Arms` 30980121 uses `77208#7` for "target is in
    /// [Poison] status" — Poison buffs all share `typeId 6003`
    /// whose `includeTypes` is `"7"`.
    HasBuffGroup { group: i32 },
    /// Inverse of `HasBuffGroup`. Encoded as `78208#group`.
    NoBuffGroup { group: i32 },

    LifeLess { threshold_permille: i32 },
    LifeMore { threshold_permille: i32 },

    TargetCareer { career_ids: Vec<i32> },
    UseExSkill,
    UseSkillId,
    TriggerBullet,
    TeammateInjuryCountNotReset { threshold: i32 },
    BeAttacked,
    BloodPool,
    HurtNotRestraint,
    ExpointMoreThan { threshold: i32 },
    ExpointLessThan { threshold: i32 },
    Random { permille: i32 },
    CanUseSkill,
    TeamInjuryCountRound,
    TeammateInjuryCount { threshold: i32 },
    PowerCompare,
    HeroRoundInterval { start_round: i32, period: i32 },
    Dead,
    MultiHpXIn,
    TeammateAlive { expect_dead: bool },
    ActiveUseSkill,
    ActiveUseSkillId { skill_ids: Vec<i32> },
    PerBuffIdCount { buff_ids: Vec<i32> },
    ExSkillLevel { levels: Vec<i32> },
    InMagicCircleId { circle_id: i32 },
    HurtNumType { type_id: i32 },
    HurtRestraint,
    NoActRound,
    PerExPoint { threshold: i32 },
    PerDecrExPoint { threshold: i32 },
    PerHasTargetCareerList { careers: Vec<i32> },
    TeammateUseExSkill,
    CareerCheck { subtype_id: i32, param: i32 },
    BattleTagNum { tag_id: i32, threshold: i32 },
    BloodPoolMax { min: i32, max: i32 },
    TargetCount { value: i32, mode: i32 },

    Unknown { raw: String },
}
