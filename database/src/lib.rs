use sqlx::Row;
use sqlx::migrate;
use sqlx::migrate::MigrateError;
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

mod config;
pub mod db;
pub mod models;

pub use config::DatabaseSettings;
pub use sqlx::{Error, SqlitePool, query, query_as};

pub async fn connect_to(settings: &DatabaseSettings) -> sqlx::Result<SqlitePool> {
    ensure_database_exists(&settings.db_name)?;

    SqlitePool::connect(&settings.to_string()).await
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), migrate::MigrateError> {
    info!("Running database migrations...");
    migrate!("./migrations").run(pool).await?;
    info!("Migrations completed successfully");

    // Guard: ensure pre_round_hand column exists in battle_replays.
    // This handles the case where the DB was created before migration 044 was added,
    // and the binary was built with a stale cache that didn't embed the new file.
    let _ = ensure_battle_replays_columns(pool).await;

    Ok(())
}

/// Adds any missing columns to battle_replays that may not have been applied
/// by migration 044 due to build caching or a pre-existing DB.
async fn ensure_battle_replays_columns(pool: &SqlitePool) -> anyhow::Result<()> {
    // Check if pre_round_hand column already exists
    let cols: Vec<(i32, String, String, i32, Option<String>, i32)> =
        sqlx::query_as("PRAGMA table_info(battle_replays)")
            .fetch_all(pool)
            .await?;

    let has_pre_round_hand = cols.iter().any(|(_, name, _, _, _, _)| name == "pre_round_hand");

    if !has_pre_round_hand {
        info!("battle_replays.pre_round_hand column missing — adding it now");
        sqlx::query(
            "ALTER TABLE battle_replays ADD COLUMN pre_round_hand TEXT NOT NULL DEFAULT '[]'",
        )
        .execute(pool)
        .await?;
        info!("battle_replays.pre_round_hand column added successfully");
    }

    Ok(())
}

/// Runs migrations, and if a checksum mismatch is detected (because a migration file was edited
/// in-place), backs up all user data, recreates the DB from scratch, and restores the data.
/// New columns introduced by the edited migration get their DEFAULT values automatically.
pub async fn migrate_or_rescue(settings: &DatabaseSettings) -> anyhow::Result<SqlitePool> {
    let pool = connect_to(settings).await?;

    match run_migrations(&pool).await {
        Ok(()) => return Ok(pool),
        Err(MigrateError::VersionMismatch(v)) => {
            warn!("Migration checksum mismatch at version {v} — starting DB rescue");
        }
        Err(e) => return Err(e.into()),
    }

    let backup = dump_all_tables(&pool).await?;
    pool.close().await;

    let bak_path = format!("{}.bak", settings.db_name);
    std::fs::rename(&settings.db_name, &bak_path)
        .map_err(|e| anyhow::anyhow!("Failed to rename DB to .bak: {e}"))?;
    info!("Old DB saved as {bak_path}, recreating schema...");

    let pool = connect_to(settings).await?;
    run_migrations(&pool).await?;

    restore_all_tables(&pool, backup).await?;
    info!("DB rescue complete");

    Ok(pool)
}

#[derive(Debug, Clone)]
enum SqlVal {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

// table name → rows, each row is ordered (col_name, value) pairs
type Dump = HashMap<String, Vec<Vec<(String, SqlVal)>>>;

async fn dump_all_tables(pool: &SqlitePool) -> anyhow::Result<Dump> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND name NOT LIKE '_sqlx_%'
         ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    let mut dump: Dump = HashMap::new();

    for table in &tables {
        // PRAGMA table_info returns: cid(i32), name(String), type(String),
        //                            notnull(i32), dflt_value(Option<String>), pk(i32)
        let col_types: Vec<(i32, String, String, i32, Option<String>, i32)> =
            sqlx::query_as(&format!("PRAGMA table_info(\"{}\")", table))
                .fetch_all(pool)
                .await?;
        // Keep only (name, declared_type) pairs
        let col_types: Vec<(String, String)> = col_types
            .into_iter()
            .map(|(_, name, typ, _, _, _)| (name, typ))
            .collect();

        let rows = sqlx::query(&format!("SELECT * FROM \"{}\"", table))
            .fetch_all(pool)
            .await?;

        let mut table_rows = Vec::with_capacity(rows.len());

        for row in &rows {
            let mut cols = Vec::with_capacity(col_types.len());
            for (col_name, col_type) in &col_types {
                let idx = col_name.as_str();
                let affinity = col_type.to_uppercase();

                let val = if affinity.contains("INT") || affinity.contains("BOOL") {
                    match row.try_get::<Option<i64>, _>(idx) {
                        Ok(Some(v)) => SqlVal::Int(v),
                        Ok(None) => SqlVal::Null,
                        Err(_) => decode_fallback(row, idx),
                    }
                } else if affinity.contains("REAL")
                    || affinity.contains("FLOA")
                    || affinity.contains("DOUB")
                {
                    match row.try_get::<Option<f64>, _>(idx) {
                        Ok(Some(v)) => SqlVal::Real(v),
                        Ok(None) => SqlVal::Null,
                        Err(_) => decode_fallback(row, idx),
                    }
                } else if affinity.contains("BLOB") {
                    match row.try_get::<Option<Vec<u8>>, _>(idx) {
                        Ok(Some(v)) => SqlVal::Blob(v),
                        Ok(None) => SqlVal::Null,
                        Err(_) => decode_fallback(row, idx),
                    }
                } else {
                    // TEXT / VARCHAR / JSON / anything else
                    match row.try_get::<Option<String>, _>(idx) {
                        Ok(Some(v)) => SqlVal::Text(v),
                        Ok(None) => SqlVal::Null,
                        Err(_) => decode_fallback(row, idx),
                    }
                };

                cols.push((col_name.clone(), val));
            }
            table_rows.push(cols);
        }

        info!("Backed up {} rows from `{}`", table_rows.len(), table);
        dump.insert(table.clone(), table_rows);
    }

    Ok(dump)
}

fn decode_fallback(row: &sqlx::sqlite::SqliteRow, col: &str) -> SqlVal {
    if let Ok(Some(v)) = row.try_get::<Option<i64>, _>(col) {
        return SqlVal::Int(v);
    }
    if let Ok(Some(v)) = row.try_get::<Option<f64>, _>(col) {
        return SqlVal::Real(v);
    }
    if let Ok(Some(v)) = row.try_get::<Option<String>, _>(col) {
        return SqlVal::Text(v);
    }
    if let Ok(Some(v)) = row.try_get::<Option<Vec<u8>>, _>(col) {
        return SqlVal::Blob(v);
    }
    SqlVal::Null
}

async fn restore_all_tables(pool: &SqlitePool, dump: Dump) -> anyhow::Result<()> {
    // Use a single dedicated connection so PRAGMA foreign_keys = OFF applies to every INSERT.
    let mut conn = pool.acquire().await?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await?;

    for (table, rows) in &dump {
        if rows.is_empty() {
            continue;
        }

        let mut restored = 0usize;
        let mut skipped = 0usize;

        for row in rows {
            if row.is_empty() {
                continue;
            }

            let col_list = row
                .iter()
                .map(|(c, _)| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ");
            let placeholders = row.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

            let sql = format!(
                "INSERT OR IGNORE INTO \"{}\" ({}) VALUES ({})",
                table, col_list, placeholders
            );

            let mut q = sqlx::query(&sql);
            for (_, val) in row {
                q = match val {
                    SqlVal::Null => q.bind(None::<i64>),
                    SqlVal::Int(v) => q.bind(v),
                    SqlVal::Real(v) => q.bind(v),
                    SqlVal::Text(v) => q.bind(v),
                    SqlVal::Blob(v) => q.bind(v.as_slice()),
                };
            }

            match q.execute(&mut *conn).await {
                Ok(_) => restored += 1,
                Err(e) => {
                    warn!("Skipping row in `{table}`: {e}");
                    skipped += 1;
                }
            }
        }

        info!("Restored `{table}`: {restored} rows ({skipped} skipped)");
    }

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await?;

    Ok(())
}

fn ensure_database_exists(db_path: &str) -> sqlx::Result<()> {
    let path = Path::new(db_path);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(e))?;
        info!("Ensured database directory exists: {}", parent.display());
    }

    if !path.exists() {
        std::fs::File::create(path).map_err(|e| Error::Io(e))?;
        info!("Created new database file: {}", db_path);
    } else {
        info!("Using existing database: {}", db_path);
    }

    Ok(())
}