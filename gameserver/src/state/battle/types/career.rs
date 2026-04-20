#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum CareerType {
    // types also does 30% less dmg to their counters
    // no dmg bonus to non counters
    Mineral = 1, // does 30% bonus damage to beast
    Star = 2,    // does 30% bonus damage to mineral
    Plant = 3,   // does 30% bonus damage to star
    Beast = 4,   // does 30% bonus damage to plants

    // does 30% bonus damage to each other
    // no dmg bonus to other types
    Spirit = 5,
    Intellect = 6,
}

impl CareerType {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Mineral),
            2 => Some(Self::Star),
            3 => Some(Self::Plant),
            4 => Some(Self::Beast),
            5 => Some(Self::Spirit),
            6 => Some(Self::Intellect),
            _ => None,
        }
    }
}

impl From<CareerType> for i32 {
    fn from(career: CareerType) -> Self {
        career as i32
    }
}

impl From<i32> for CareerType {
    fn from(value: i32) -> Self {
        match value {
            1 => Self::Mineral,
            2 => Self::Star,
            3 => Self::Plant,
            4 => Self::Beast,
            5 => Self::Spirit,
            6 => Self::Intellect,
            _ => Self::Mineral,
        }
    }
}
