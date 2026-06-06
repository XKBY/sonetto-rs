use crate::network::handler;
use crate::state::ConnectionContext;
use byteorder::{BE, ByteOrder};
use std::sync::Arc;
use tokio::{io::AsyncReadExt, sync::Mutex};

pub async fn handle_client(ctx: Arc<Mutex<ConnectionContext>>) -> anyhow::Result<()> {
    loop {
        let packet = {
            let conn = ctx.lock().await;
            let mut socket = conn.socket.lock().await;

            let mut header = [0u8; 4];
            if let Err(e) = socket.read_exact(&mut header).await {
                tracing::debug!("Client disconnected: {e}");
                return Ok(());
            }

            let packet_len = BE::read_i32(&header) as usize;
            let mut buffer = vec![0u8; packet_len];
            if let Err(e) = socket.read_exact(&mut buffer).await {
                tracing::warn!("Failed to read packet body ({} bytes): {e}", packet_len);
                return Ok(());
            }

            let mut packet = Vec::with_capacity(4 + packet_len);
            packet.extend_from_slice(&header);
            packet.extend_from_slice(&buffer);
            packet
        };

        if let Err(e) = handler::dispatch_command(ctx.clone(), &packet[..]).await {
            // Check if this is a fatal error (decode failure, auth) or just an unhandled cmd.
            // For unhandled/unknown commands, log and continue so the session stays alive
            // and we can observe what the client sends next.
            use crate::error::{AppError, CmdError};
            let is_fatal = !matches!(&e, AppError::Cmd(CmdError::UnhandledCmd(_)) | AppError::Cmd(CmdError::UnregisteredCmd(_)));
            if is_fatal {
                tracing::error!("Dispatch error (fatal): {e}");
                break;
            } else {
                tracing::warn!("Dispatch error (non-fatal, continuing): {e}");
            }
        }

        {
            let mut conn = ctx.lock().await;
            if let Err(e) = conn.flush_send_queue().await {
                tracing::error!("Failed to flush send queue: {e}");
                break;
            }
        }
    }

    Ok(())
}