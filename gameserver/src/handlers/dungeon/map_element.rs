use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use database::db::game::dungeons::finish_element;
use prost::Message;
use sonettobuf::{ChapterMapElementUpdatePush, CmdId, MapElementReply, MapElementRequest};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_map_element(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let request = MapElementRequest::decode(&req.data[..])?;

    let element_id = request.element_id.unwrap_or(0);

    tracing::info!(
        "MapElement: element_id={} dialog_ids={:?}",
        element_id,
        request.dialog_ids,
    );

    let (player_id, pool) = {
        let conn = ctx.lock().await;
        (conn.player_id.ok_or(AppError::NotLoggedIn)?, conn.state.db.clone())
    };

    if element_id != 0 {
        if let Err(e) = finish_element(&pool, player_id, element_id).await {
            tracing::warn!("MapElement: finish_element failed (non-fatal): {}", e);
        }
    }

    {
        let mut conn = ctx.lock().await;
        conn.notify(
            CmdId::ChapterMapElementUpdatePushCmd,
            ChapterMapElementUpdatePush {
                elements: if element_id != 0 { vec![element_id] } else { vec![] },
            },
        )
            .await?;
    }

    let reply = MapElementReply {
        element_id: request.element_id,
        dialog_ids: request.dialog_ids,
        record: request.record,
    };

    {
        let mut conn = ctx.lock().await;
        conn.send_reply(CmdId::MapElementCmd, reply, 0, req.up_tag).await?;
    }

    Ok(())
}
