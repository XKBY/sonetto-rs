use crate::network::packet::ClientPacket;
use crate::state::{ConnectionContext, handle_dungeon_end};
use crate::{error::AppError, state::send_end_fight_push};
use prost::Message;
use sonettobuf::{CmdId, EndDungeonReply, EndDungeonRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_dungeon_end_dungeon(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = EndDungeonRequest::decode(&req.data[..])?;

    let is_abort = request.is_abort.ok_or(AppError::InvalidRequest)?;

    tracing::info!("Dungeon ended with is_abort: {}", is_abort);

    let battle_opt = {
        let conn = ctx.lock().await;
        conn.active_battle.clone()
    };

    if let Some(battle) = battle_opt {
        // Normal path: FightEndFightCmd was not sent (e.g. pure-story dungeon,
        // or the client sends DungeonEndDungeon without a preceding FightEndFight).
        let (fight_group, is_replay, chapter_id, episode_id, multiplication, player_id, pool, battle_id) = {
            (
                battle.fight_group.clone(),
                battle.is_replay.unwrap_or(false),
                battle.chapter_id,
                battle.episode_id,
                battle.multiplication.unwrap_or(1),
                {
                    let conn = ctx.lock().await;
                    conn.player_id.ok_or(AppError::NotLoggedIn)?
                },
                {
                    let conn = ctx.lock().await;
                    conn.state.db.clone()
                },
                battle.fight_id.unwrap_or_default(),
            )
        };

        if is_abort {
            send_end_fight_push(
                ctx.clone(),
                battle_id,
                0,
                fight_group.clone().unwrap_or_default(),
                vec![],
                vec![],
                !is_replay,
            )
            .await?;
        }

        let is_victory = !is_abort;
        handle_dungeon_end(
            ctx.clone(),
            &pool,
            player_id,
            chapter_id,
            episode_id,
            &fight_group,
            multiplication,
            is_replay,
            is_victory,
        )
        .await?;

        let mut conn = ctx.lock().await;
        conn.active_battle = None;
    } else {
        // active_battle is None: the fight was already fully processed by
        // on_fight_end_fight (which calls handle_dungeon_end and clears the
        // battle). This is the normal sequence for combat dungeons:
        //   FightEndFightCmd → DungeonEndDungeonCmd
        // Nothing to do here except send the reply.
        tracing::info!("DungeonEndDungeon: battle already cleared by FightEndFight, sending reply only");
    }

    let data = EndDungeonReply {};
    let mut conn = ctx.lock().await;
    conn.send_reply(CmdId::DungeonEndDungeonCmd, data, 0, req.up_tag)
        .await?;

    Ok(())
}