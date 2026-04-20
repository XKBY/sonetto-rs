#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum CardOpType {
    MoveCard = 1,
    PlayCard = 2,
    MoveUniversal = 3,
    AssistBoss = 4,
    Season2ChangeHero = 5,
    PlayerFinisherSkill = 6,
    BloodPool = 7,
    SimulateDissolveCard = -99,
    Rouge2Music = -100,
}

impl TryFrom<i32> for CardOpType {
    type Error = ();
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(Self::MoveCard),
            2 => Ok(Self::PlayCard),
            3 => Ok(Self::MoveUniversal),
            4 => Ok(Self::AssistBoss),
            5 => Ok(Self::Season2ChangeHero),
            6 => Ok(Self::PlayerFinisherSkill),
            7 => Ok(Self::BloodPool),
            -99 => Ok(Self::SimulateDissolveCard),
            -100 => Ok(Self::Rouge2Music),
            _ => Err(()),
        }
    }
}
