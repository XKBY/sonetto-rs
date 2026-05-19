use super::super::manager::fight_data_mgr::FightDataMgr;
use anyhow::Result;
use sonettobuf::{Fight, FightRound};

pub async fn build_initial_round(
    fight: Fight,
    battle_id: i32,
) -> Result<(FightRound, FightDataMgr)> {
    let mut fight_mgr = FightDataMgr::new(fight);
    let round = fight_mgr.build_initial_round(battle_id)?;
    Ok((round, fight_mgr))
}
