use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use crate::{error::AppError, send_reply};
use sonettobuf::{CmdId, TowerComposeGetInfoReply};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_tower_compose_get_info(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    send_reply!(
        ctx,
        req.up_tag,
        CmdId::TowerComposeGetInfoCmd,
        TowerComposeGetInfoReply,
        "tower/tower_compose_get_info.json"
    );

    Ok(())
}
