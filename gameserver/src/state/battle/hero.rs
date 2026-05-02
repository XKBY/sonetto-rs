//! Exhaustive enum of heroes that own Rust-level behavior in
//! `heroes/`. Heroes not listed here go through the generic dispatch
//! path — their kit is fully expressible from skill/buff config.
//!
//! New variant = new file in `heroes/` + a match arm in the relevant
//! phase dispatcher. Skipping either side is a compile error.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HeroId {
    Sotheby = 3009,
    Melania = 3062,
    Pickles = 3063,
    Kakania = 3080,
    Semmelweis = 3088,
    Tuesday = 3098,
    Willow = 3104,
    Recoleta = 3114,
    Nautika = 3120,
    Rubuska = 3125,
    Sentinel = 3126,
}

impl HeroId {
    pub fn from_model_id(id: i32) -> Option<Self> {
        Some(match id {
            3009 => Self::Sotheby,
            3062 => Self::Melania,
            3063 => Self::Pickles,
            3080 => Self::Kakania,
            3088 => Self::Semmelweis,
            3098 => Self::Tuesday,
            3104 => Self::Willow,
            3114 => Self::Recoleta,
            3120 => Self::Nautika,
            3125 => Self::Rubuska,
            3126 => Self::Sentinel,
            _ => return None,
        })
    }

    pub fn model_id(self) -> i32 {
        self as i32
    }
}
