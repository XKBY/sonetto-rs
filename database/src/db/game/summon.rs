use crate::models::game::summon::*;
use anyhow::Result;
use chrono::{NaiveDateTime, TimeZone, Utc};
use common::time::ServerTime;
use config::store_recommend::StoreRecommend;
use sonettobuf::SummonResult;
use sqlx::SqlitePool;
use std::collections::HashMap;

pub async fn get_summon_stats(pool: &SqlitePool, user_id: i64) -> Result<UserSummonStats> {
    let stats =
        sqlx::query_as::<_, UserSummonStats>("SELECT * FROM user_summon_stats WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;

    Ok(stats.unwrap_or(UserSummonStats {
        user_id,
        free_equip_summon: false,
        is_show_new_summon: false,
        new_summon_count: 0,
        total_summon_count: 0,
    }))
}

pub async fn get_summon_pool_infos(pool: &SqlitePool, user_id: i64) -> Result<Vec<SummonPoolInfo>> {
    // All banners ever configured — mirrors what the real server sends
    let banners = sqlx::query_as::<_, BannerSchedule>(
        "SELECT pool_id, online_time, offline_time, created_at, updated_at
         FROM banner_schedule ORDER BY pool_id",
    )
    .fetch_all(pool)
    .await?;

    if banners.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load all user pool rows in one query
    let user_pools: HashMap<i32, UserSummonPool> =
        sqlx::query_as::<_, UserSummonPool>("SELECT * FROM user_summon_pools WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|p| (p.pool_id, p))
            .collect();

    // Batch-load lucky bags
    let lucky_bags = load_all_lucky_bags(pool, user_id).await?;

    // Batch-load sp pool base rows
    let sp_pools = load_all_sp_pools(pool, user_id).await?;

    // Batch-load pop-up infos
    let all_pop_up_infos = load_all_pop_up_infos(pool, user_id).await?;

    let now = ServerTime::now_ms();

    let result = banners
        .into_iter()
        .map(|banner| {
            let pool_data = user_pools.get(&banner.pool_id).cloned().unwrap_or({
                UserSummonPool {
                    id: 0,
                    user_id,
                    pool_id: banner.pool_id,
                    online_time: banner.online_time,
                    offline_time: banner.offline_time,
                    have_free: false,
                    used_free_count: 0,
                    discount_time: 0,
                    can_get_guarantee_sr_count: 0,
                    guarantee_sr_countdown: 0,
                    summon_count: 0,
                    have_free10_count: 0,
                    not_ssr_count: 0,
                    total_free10_use_count: 0,
                    created_at: now,
                    updated_at: now,
                }
            });

            SummonPoolInfo {
                lucky_bag: lucky_bags.get(&banner.pool_id).cloned(),
                sp_pool: sp_pools.get(&banner.pool_id).cloned(),
                pop_up_infos: all_pop_up_infos
                    .get(&banner.pool_id)
                    .cloned()
                    .unwrap_or_default(),
                pool: pool_data,
            }
        })
        .collect();

    Ok(result)
}

/// Derives banner schedule from store_recommend config entries.
/// Entries where relations = "1#<pool_id>" and the pool exists in summon_pool config are synced.
pub async fn sync_banner_schedule_from_config(
    db: &SqlitePool,
    store_recommends: &[StoreRecommend],
    summon_pool_ids: &std::collections::HashSet<i32>,
) -> anyhow::Result<()> {
    let now = Utc::now().timestamp() as i32;

    for rec in store_recommends {
        if rec.is_offline != 0 {
            continue;
        }
        let pool_id = match parse_pool_relation(&rec.relations) {
            Some(id) if summon_pool_ids.contains(&id) => id,
            _ => continue,
        };
        let (online_time, offline_time) = match (
            parse_ts_seconds(&rec.online_time),
            parse_ts_seconds(&rec.offline_time),
        ) {
            (Some(on), Some(off)) => (on, off),
            _ => continue,
        };

        sqlx::query(
            r#"
            INSERT INTO banner_schedule (pool_id, online_time, offline_time, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(pool_id) DO UPDATE SET
                online_time  = excluded.online_time,
                offline_time = excluded.offline_time,
                updated_at   = excluded.updated_at
            "#,
        )
        .bind(pool_id)
        .bind(online_time)
        .bind(offline_time)
        .bind(now)
        .bind(now)
        .execute(db)
        .await?;
    }

    Ok(())
}

fn parse_pool_relation(relations: &str) -> Option<i32> {
    relations.strip_prefix("1#")?.parse::<i32>().ok()
}

fn parse_ts_seconds(s: &str) -> Option<i32> {
    let dt = NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    Some(Utc.from_utc_datetime(&dt).timestamp() as i32)
}

async fn load_all_lucky_bags(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<HashMap<i32, LuckyBagInfo>> {
    let bags: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT pool_id, count, not_ssr_count FROM user_lucky_bags WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let single_bags: Vec<(i32, i32, bool)> = sqlx::query_as(
        "SELECT pool_id, bag_id, is_open FROM user_single_bags WHERE user_id = ? ORDER BY bag_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut singles_by_pool: HashMap<i32, Vec<SingleBagInfo>> = HashMap::new();
    for (pid, bag_id, is_open) in single_bags {
        singles_by_pool
            .entry(pid)
            .or_default()
            .push(SingleBagInfo { bag_id, is_open });
    }

    Ok(bags
        .into_iter()
        .map(|(pool_id, count, not_ssr_count)| {
            (
                pool_id,
                LuckyBagInfo {
                    count,
                    not_ssr_count,
                    single_bag_infos: singles_by_pool.remove(&pool_id).unwrap_or_default(),
                },
            )
        })
        .collect())
}

async fn load_all_sp_pools(pool: &SqlitePool, user_id: i64) -> Result<HashMap<i32, SpPoolInfo>> {
    let rows: Vec<(i32, i32, i32, i32, i64, bool, i32)> = sqlx::query_as(
        "SELECT pool_id, sp_type, limited_ticket_id, limited_ticket_num,
                open_time, used_first_ssr_guarantee, infallible_item_status
         FROM user_sp_pool_info WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let up_heroes: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT pool_id, hero_id FROM user_sp_pool_up_heroes WHERE user_id = ? ORDER BY hero_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let reward_progresses: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT pool_id, progress_id FROM user_sp_pool_reward_progress WHERE user_id = ? ORDER BY progress_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut heroes_by_pool: HashMap<i32, Vec<i32>> = HashMap::new();
    for (pid, hero_id) in up_heroes {
        heroes_by_pool.entry(pid).or_default().push(hero_id);
    }

    let mut progress_by_pool: HashMap<i32, Vec<i32>> = HashMap::new();
    for (pid, progress_id) in reward_progresses {
        progress_by_pool.entry(pid).or_default().push(progress_id);
    }

    Ok(rows
        .into_iter()
        .map(
            |(
                pool_id,
                sp_type,
                limited_ticket_id,
                limited_ticket_num,
                open_time,
                used_first_ssr_guarantee,
                infallible_item_status,
            )| {
                (
                    pool_id,
                    SpPoolInfo {
                        sp_type,
                        limited_ticket_id,
                        limited_ticket_num,
                        open_time: open_time as u64,
                        used_first_ssr_guarantee,
                        infallible_item_status,
                        up_hero_ids: heroes_by_pool.remove(&pool_id).unwrap_or_default(),
                        has_get_reward_progresses: progress_by_pool
                            .remove(&pool_id)
                            .unwrap_or_default(),
                    },
                )
            },
        )
        .collect())
}

async fn load_all_pop_up_infos(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<HashMap<i32, Vec<PopUpInfo>>> {
    let rows: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT pool_id, order_id, recommend_pop_up_count
         FROM user_pool_pop_up_infos WHERE user_id = ? ORDER BY pool_id, order_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<i32, Vec<PopUpInfo>> = HashMap::new();
    for (pool_id, order_id, recommend_pop_up_count) in rows {
        map.entry(pool_id).or_default().push(PopUpInfo {
            order_id,
            recommend_pop_up_count,
        });
    }
    Ok(map)
}

pub async fn get_sp_pool_info(
    pool: &SqlitePool,
    user_id: i64,
    pool_id: i32,
) -> Result<Option<SpPoolInfo>> {
    let sp_data: Option<(i32, i32, i32, i64, bool, i32)> = sqlx::query_as(
        "SELECT sp_type, limited_ticket_id, limited_ticket_num, open_time, used_first_ssr_guarantee, infallible_item_status
         FROM user_sp_pool_info WHERE user_id = ? AND pool_id = ?",
    )
    .bind(user_id)
    .bind(pool_id)
    .fetch_optional(pool)
    .await?;

    if let Some((
        sp_type,
        limited_ticket_id,
        limited_ticket_num,
        open_time,
        used_first_ssr_guarantee,
        infallible_item_status,
    )) = sp_data
    {
        let up_hero_ids = sqlx::query_scalar(
            "SELECT hero_id FROM user_sp_pool_up_heroes WHERE user_id = ? AND pool_id = ? ORDER BY hero_id"
        )
        .bind(user_id)
        .bind(pool_id)
        .fetch_all(pool)
        .await?;

        let has_get_reward_progresses = sqlx::query_scalar(
            "SELECT progress_id FROM user_sp_pool_reward_progress WHERE user_id = ? AND pool_id = ? ORDER BY progress_id"
        )
        .bind(user_id)
        .bind(pool_id)
        .fetch_all(pool)
        .await?;

        Ok(Some(SpPoolInfo {
            sp_type,
            up_hero_ids,
            limited_ticket_id,
            limited_ticket_num,
            open_time: open_time as u64,
            used_first_ssr_guarantee,
            has_get_reward_progresses,
            infallible_item_status,
        }))
    } else {
        Ok(None)
    }
}

pub async fn add_summon_history(
    pool: &SqlitePool,
    user_id: i64,
    pool_id: i32,
    pool_name: &str,
    pool_type: i32,
    summon_type: i32,
    results: &[SummonResult],
) -> sqlx::Result<()> {
    let now = common::time::ServerTime::now_ms();

    // Insert summon history row
    let history_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO user_summon_history (
            user_id, pool_id, summon_type, pool_type, pool_name, summon_time
        )
        VALUES (?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(pool_id)
    .bind(summon_type)
    .bind(pool_type)
    .bind(pool_name)
    .bind(now)
    .fetch_one(pool)
    .await?;

    // Insert gained items (heroes from results)
    for (idx, result) in results.iter().enumerate() {
        if let Some(hero_id) = result.hero_id {
            // Insert hero result
            sqlx::query(
                r#"
                INSERT INTO user_summon_history_items (
                    history_id, result_index, gain_id
                )
                VALUES (?, ?, ?)
                "#,
            )
            .bind(history_id)
            .bind(idx as i32)
            .bind(hero_id)
            .execute(pool)
            .await?;
        }
    }

    tracing::debug!(
        "Inserted summon history for user {}: pool {}, {} results",
        user_id,
        pool_id,
        results.len()
    );

    Ok(())
}

pub async fn update_sp_pool_up_heroes(
    pool: &SqlitePool,
    user_id: i64,
    pool_id: i32,
    up_hero_ids: &[i32],
) -> Result<()> {
    // Ensure a base sp_pool_info row exists so load_all_sp_pools can find these up heroes.
    sqlx::query(
        "INSERT OR IGNORE INTO user_sp_pool_info
         (user_id, pool_id, sp_type, limited_ticket_id, limited_ticket_num,
          open_time, used_first_ssr_guarantee, infallible_item_status)
         VALUES (?, ?, 0, 0, 0, 0, 0, 0)",
    )
    .bind(user_id)
    .bind(pool_id)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        DELETE FROM user_sp_pool_up_heroes
        WHERE user_id = ? AND pool_id = ?
        "#,
    )
    .bind(user_id)
    .bind(pool_id)
    .execute(pool)
    .await?;

    for hero_id in up_hero_ids {
        sqlx::query(
            r#"
            INSERT INTO user_sp_pool_up_heroes (user_id, pool_id, hero_id)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(user_id)
        .bind(pool_id)
        .bind(*hero_id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn use_discount(pool: &SqlitePool, user_id: i64, pool_id: i32) -> Result<()> {
    let now = common::time::ServerTime::now_ms();

    sqlx::query(
        "UPDATE user_summon_pools
         SET discount_time = discount_time - 1, updated_at = ?
         WHERE user_id = ? AND pool_id = ? AND discount_time > 0",
    )
    .bind(now)
    .bind(user_id)
    .bind(pool_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn increment_summon_count(
    pool: &SqlitePool,
    user_id: i64,
    pool_id: i32,
    count: i32,
) -> Result<()> {
    let now = common::time::ServerTime::now_ms();
    let game_data = config::configs::get();
    let summon_pool = game_data
        .summon_pool
        .iter()
        .find(|p| p.id == pool_id)
        .ok_or_else(|| anyhow::anyhow!("Summon pool {} not found", pool_id))?;

    let pool_type = summon_pool.r#type;

    if pool_type == 3 {
        let type_3_pool_ids: Vec<i32> = game_data
            .summon_pool
            .iter()
            .filter(|p| p.r#type == 3)
            .map(|p| p.id)
            .collect();

        let mut tx = pool.begin().await?;

        for type_3_pool_id in type_3_pool_ids {
            sqlx::query(
                "INSERT INTO user_summon_pools (user_id, pool_id, offline_time, summon_count, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT(user_id, pool_id) DO UPDATE SET
                     summon_count = summon_count + ?,
                     updated_at = ?"
            )
            .bind(user_id)
            .bind(type_3_pool_id)
            .bind(1750327199)
            .bind(count)
            .bind(now)
            .bind(now)
            .bind(count)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
    } else {
        sqlx::query(
            "INSERT INTO user_summon_pools (user_id, pool_id, summon_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(user_id, pool_id) DO UPDATE SET
                 summon_count = summon_count + ?,
                 updated_at = ?",
        )
        .bind(user_id)
        .bind(pool_id)
        .bind(count)
        .bind(now)
        .bind(now)
        .bind(count)
        .bind(now)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn get_banner_schedule(
    db: &SqlitePool,
    pool_id: i32,
) -> anyhow::Result<Option<BannerSchedule>> {
    let result = sqlx::query_as::<_, BannerSchedule>(
        "SELECT pool_id, online_time, offline_time, created_at, updated_at
         FROM banner_schedule
         WHERE pool_id = ?",
    )
    .bind(pool_id)
    .fetch_optional(db)
    .await?;

    Ok(result)
}
