use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use common::{config as server_config, excel_data_directory, init_config};
use gameserver::state::init_skill_cache;

pub fn bootstrap_game_data(config_path: &Path, excel_override: Option<PathBuf>) -> Result<()> {
    let mut cfg = server_config::ServerConfig::load_or_create(&config_path.to_path_buf())
        .with_context(|| format!("failed to load/create {}", config_path.display()))?;

    let config_dir = config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    cfg.resolve_paths(&config_dir)?;
    init_config(cfg);

    let excel_path = excel_override.unwrap_or_else(|| excel_data_directory().clone());

    if !excel_path.exists() {
        bail!(
            "Excel data directory not found: {} (pass --excel <path> to override)",
            excel_path.display()
        );
    }

    ::config::configs::init(
        excel_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("excel data path is not valid UTF-8"))?,
    )?;
    init_skill_cache();
    Ok(())
}
