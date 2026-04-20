use crate::error::AppError;
use crate::network::packet::ClientPacket;
use crate::state::ConnectionContext;
use sonettobuf::{BpScoreBonusInfo, CmdId, GetBpInfoReply, Task};
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn on_get_bp_info(
    ctx: Arc<Mutex<ConnectionContext>>,
    req: ClientPacket,
) -> Result<(), AppError> {
    let game_data = config::configs::get();

    let max_id = game_data
        .bp_task
        .iter()
        .map(|task| task.bp_id)
        .filter(|&id| id != 305)
        .max();

    let max_tasks: Vec<_> = if let Some(max_id) = max_id {
        game_data
            .bp_task
            .iter()
            .filter(|task| task.bp_id == max_id)
            .collect()
    } else {
        Vec::new()
    };

    let (start_time, end_time): (Option<i32>, Option<i32>) = game_data
        .bp_task
        .iter()
        .filter(|t| Some(t.bp_id) == max_id)
        .map(|t| (parse_unix(&t.start_time), parse_unix(&t.end_time)))
        .fold((None, None), |(min_s, max_e), (s, e)| {
            (
                Some(min_s.map_or(s, |v| v.min(s))),
                Some(max_e.map_or(e, |v| v.max(e))),
            )
        });

    let task_info: Vec<Task> = max_tasks
        .clone()
        .into_iter()
        .map(|task| Task {
            id: task.id,
            progress: task.max_progress,
            has_finished: true,
            finish_count: Some(0),
            r#type: Some(10),
            expiry_time: Some(parse_unix(&task.end_time)),
        })
        .collect();

    let score_bonus_info: Vec<BpScoreBonusInfo> = (1..=10)
        .map(|level| BpScoreBonusInfo {
            level: Some(level),
            has_getfree_bonus: Some(true),
            has_get_pay_bonus: Some(true),
            has_get_spfree_bonus: Some(false),
            has_get_sp_pay_bonus: Some(false),
        })
        .collect();

    let reply = GetBpInfoReply {
        id: max_id,
        score: Some(1000),
        pay_status: Some(2),
        start_time,
        end_time,
        task_info,
        score_bonus_info,
        weekly_score: Some(0),
        first_show: Some(false),
        has_get_self_select_bonus: vec![],
        sp_first_show: Some(true),
    };

    let mut conn = ctx.lock().await;
    conn.send_reply(CmdId::GetBpInfoCmd, reply, 0, req.up_tag)
        .await?;

    Ok(())
}

use chrono::{NaiveDateTime, TimeZone, Utc};

fn parse_unix(s: &str) -> i32 {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|naive| Utc.from_utc_datetime(&naive).timestamp() as i32)
        .unwrap_or(0)
}
