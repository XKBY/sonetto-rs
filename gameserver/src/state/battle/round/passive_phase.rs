#[derive(Debug, Clone)]
pub enum PhaseScope {
    Attackers,
    Defenders,
}

#[derive(Debug, Clone, Copy)]
pub enum PhaseDepth {
    FirstMatch,
    AllMatches,
}

#[derive(Debug, Clone, Copy)]
pub enum PhaseSkillSet {
    CombatReactive,
    BattleRuleOnly,
    ExcludeBattleRule,
    DefenderBootstrap,
}

#[derive(Debug, Clone, Copy)]
pub enum PhaseStepShape {
    FlatIfAllUpdate,
    Raw,
}

#[derive(Debug, Clone)]
pub struct PassivePhaseConfig {
    pub scope: PhaseScope,
    pub depth: PhaseDepth,
    pub skill_set: PhaseSkillSet,
    pub step_shape: PhaseStepShape,
}
