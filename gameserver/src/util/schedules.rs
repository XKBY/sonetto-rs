use config::configs;
use database::db::game::summon::sync_banner_schedule_from_config;
use sqlx::SqlitePool;
use std::collections::HashSet;

pub async fn init(db: &SqlitePool) -> anyhow::Result<()> {
    let cfg = configs::get();

    let pool_ids: HashSet<i32> = cfg.summon_pool.iter().map(|p| p.id).collect();
    let store_recs: Vec<_> = cfg.store_recommend.iter().cloned().collect();
    sync_banner_schedule_from_config(db, &store_recs, &pool_ids).await?;

    Ok(())
}
