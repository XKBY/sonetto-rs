/// Buff layer/halo type — stored in buff.type field.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffLayerType {
    Normal = 0,
    LayerMasterHalo = 1,
    LayerSlaveHalo = 2,
}

#[allow(dead_code)]
impl BuffLayerType {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Normal),
            1 => Some(Self::LayerMasterHalo),
            2 => Some(Self::LayerSlaveHalo),
            _ => None,
        }
    }
}

/// Buff alignment — used by Disperse and Purify.
/// Source: FightEnum.FightBuffType
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffAlignment {
    Bad = 1,
    Good = 2,
    Normal = 3,
}

#[allow(dead_code)]
impl BuffAlignment {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            1 => Some(Self::Bad),
            2 => Some(Self::Good),
            3 => Some(Self::Normal),
            _ => None,
        }
    }
}

/// Stack types — stored in skill_bufftype.includeTypes.
/// Source: FightEnum.BuffIncludeTypes
pub mod stack_type {

    pub fn is_stackable(include_types: &str) -> bool {
        include_types
            // includeTypes can be encoded as plain ids ("10")
            // or id-with-arg forms ("10#3"). Treat either as stackable ids.
            .split([',', '，', '#'])
            .any(|v| matches!(v.trim(), "10" | "12" | "14" | "15"))
    }
}

/// Source: FightEnum.BuffTypeList
pub const GOOD_BUFF_TYPES: &[i32] = &[1, 3, 5];
pub const BAD_BUFF_TYPES: &[i32] = &[2, 4, 6];

/// Buff category — maps skill_bufftype.type field.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BuffCategory {
    StatsUp    = 1,
    StatsDown  = 2,
    Counter    = 3,
    Control    = 4,
    PosStatus  = 5,
    NegStatus  = 6,
    Shield     = 7,
    Special    = 8,
    Passive    = 9,
    ForceField = 10,
    /// Passive (alternate)
    Passive2   = 11,
    /// Passive (alternate)
    Passive3   = 12,
    /// Psychube
    Psychube   = 13,
    Channel    = 14,
}

#[allow(dead_code)]
impl BuffCategory {
    pub fn from(val: i32) -> Option<Self> {
        match val {
            1  => Some(Self::StatsUp),
            2  => Some(Self::StatsDown),
            3  => Some(Self::Counter),
            4  => Some(Self::Control),
            5  => Some(Self::PosStatus),
            6  => Some(Self::NegStatus),
            7  => Some(Self::Shield),
            8  => Some(Self::Special),
            9  => Some(Self::Passive),
            10 => Some(Self::ForceField),
            11 => Some(Self::Passive2),
            12 => Some(Self::Passive3),
            13 => Some(Self::Psychube),
            14 => Some(Self::Channel),
            _  => None,
        }
    }
}
