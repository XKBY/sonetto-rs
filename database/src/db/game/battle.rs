use anyhow::Result;
use sqlx::SqlitePool;

/// A single round's stored data used for replay.
pub struct ReplayRoundRecord {
    /// The opers sent by the client (card-index based play ops).
    pub opers: Vec<sonettobuf::BeginRoundOper>,
    /// Cloth-skill ops for the round.
    pub cloth_skill_opers: Vec<sonettobuf::UseClothSkillOperRecord>,
    /// The player's hand **before** the round started.
    /// Used to restore exact hand state on replay so card indices remain valid.
    pub pre_round_hand: Vec<sonettobuf::CardInfo>,
}

pub async fn save_round_operations(
    pool: &SqlitePool,
    user_id: i64,
    episode_id: i32,
    battle_id: i64,
    round_number: i32,
    cloth_skill_opers: Vec<sonettobuf::UseClothSkillOperRecord>,
    opers: Vec<sonettobuf::BeginRoundOper>,
    pre_round_hand: Vec<sonettobuf::CardInfo>,
) -> Result<()> {
    let cloth_json = serde_json::to_string(&cloth_skill_opers)?;
    let opers_json = serde_json::to_string(&opers)?;
    let hand_json = serde_json::to_string(&pre_round_hand)?;
    // On the first round of a new battle, delete all previous battle records
    // for this episode so they don't accumulate indefinitely and can't
    // corrupt future replay loads.
    if round_number == 1 {
        sqlx::query(
            "DELETE FROM battle_replays WHERE user_id = ? AND episode_id = ? AND battle_id != ?",
        )
        .bind(user_id)
        .bind(episode_id)
        .bind(battle_id)
        .execute(pool)
        .await?;
    }
    sqlx::query(
        "INSERT OR REPLACE INTO battle_replays
         (user_id, episode_id, battle_id, round_number, cloth_skill_opers, opers, pre_round_hand, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(episode_id)
    .bind(battle_id)
    .bind(round_number)
    .bind(cloth_json)
    .bind(opers_json)
    .bind(hand_json)
    .bind(chrono::Utc::now().timestamp())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn load_battle_replay(
    pool: &SqlitePool,
    user_id: i64,
    episode_id: i32,
) -> Result<Vec<ReplayRoundRecord>> {
    #[allow(dead_code)]
    #[derive(sqlx::FromRow)]
    struct ReplayRow {
        round_number: i32,
        cloth_skill_opers: String,
        opers: String,
        pre_round_hand: String,
    }

    let rows: Vec<ReplayRow> = sqlx::query_as(
        "SELECT round_number, cloth_skill_opers, opers, pre_round_hand
         FROM battle_replays
         WHERE user_id = ? AND episode_id = ?
         AND battle_id = (
             SELECT battle_id FROM battle_replays
             WHERE user_id = ? AND episode_id = ?
             ORDER BY created_at DESC LIMIT 1
         )
         ORDER BY round_number",
    )
    .bind(user_id)
    .bind(episode_id)
    .bind(user_id)
    .bind(episode_id)
    .fetch_all(pool)
    .await?;

    let mut records = Vec::new();
    for row in rows {
        let cloth_skill_opers: Vec<sonettobuf::UseClothSkillOperRecord> =
            serde_json::from_str(&row.cloth_skill_opers)?;
        let opers: Vec<sonettobuf::BeginRoundOper> = serde_json::from_str(&row.opers)?;
        let pre_round_hand: Vec<sonettobuf::CardInfo> =
            serde_json::from_str(&row.pre_round_hand).unwrap_or_default();

        records.push(ReplayRoundRecord {
            opers,
            cloth_skill_opers,
            pre_round_hand,
        });
    }

    Ok(records)
}