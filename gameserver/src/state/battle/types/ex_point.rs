/// The type is stored in entity.exPointType.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExPointType {
    Common = 0,
    Belief = 1,
    Synchronization = 2,
    Adrenaline = 3,
    NoExpoint = 999,
}

#[allow(dead_code)]
impl ExPointType {
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Common),
            1 => Some(Self::Belief),
            2 => Some(Self::Synchronization),
            3 => Some(Self::Adrenaline),
            999 => Some(Self::NoExpoint),
            _ => None,
        }
    }

    pub fn gains_from_standard_actions(self) -> bool {
        matches!(self, Self::Common | Self::Belief)
    }
}
