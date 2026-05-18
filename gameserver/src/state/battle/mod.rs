mod auto;
mod card;
mod passives;

pub mod context;
pub mod destiny;
pub mod dungeon_end_logic;
pub mod emission_timeline;
pub mod end_fight;
pub mod equipment;
pub mod event_queue;
pub mod types;

pub mod manager;
pub mod mechanics;
pub mod phase;
pub mod rewards;
pub mod round;
pub mod round_end_emission;
pub mod round_state;
pub mod simulator;
pub mod step_walker;
pub mod steps;

pub mod utils;

pub mod buff;
pub mod buff_actions;
pub mod entity;
pub mod fight;
pub mod fight_step;
pub mod hero;
pub mod heroes;
pub mod skill;
pub mod trigger;

use anyhow::Result;
use sonettobuf::CardInfo;
use sonettobuf::FightRound;
use sqlx::SqlitePool;

pub use auto::generate_auto_opers;
pub use card::apply_opening_deck;
pub use card::{build_enemy_deck, build_player_deck, default_max_ap, generate_ai_deck, generate_deck, generate_initial_hand};
pub use types::{behavior::BehaviorType, condition::ConditionType};

use crate::state::battle::manager::fight_data_mgr::FightDataMgr;

#[allow(dead_code)]
pub struct BattleContext {
    pub player_id: i64,
    pub chapter_id: i32,
    pub episode_id: i32,
    pub battle_id: i32,
    pub max_ap: i32,
}

pub async fn create_battle(
    pool: &SqlitePool,
    ctx: BattleContext,
    fight_group: &sonettobuf::FightGroup,
    player_deck: Vec<CardInfo>,
) -> Result<(FightRound, FightDataMgr, Vec<CardInfo>)> {
    let built = fight::builder::build_fight(pool, &ctx, fight_group).await?;

    let seed = (ctx.player_id as u64) ^ (ctx.episode_id as u64) ^ 0xA11C;

    let ai_deck = generate_ai_deck(&built.fight, seed).await;

    let (initial_round, fight_data_mgr) =
        round::build_initial_round(built.fight, player_deck, ai_deck.clone(), ctx.battle_id)
            .await?;

    Ok((initial_round, fight_data_mgr, ai_deck))
}
