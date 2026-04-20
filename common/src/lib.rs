use once_cell::sync::OnceCell;
use std::path::PathBuf;

pub mod config;

pub mod time;

static CONFIG: OnceCell<config::ServerConfig> = OnceCell::new();

pub fn init_config(config: config::ServerConfig) {
    CONFIG.set(config).expect("Config already initialized");
}

pub fn config() -> &'static config::ServerConfig {
    CONFIG
        .get()
        .expect("Config not initialized - call init_config first")
}

pub fn host() -> &'static str {
    &config().server.host
}

pub fn dns() -> &'static str {
    &config().server.dns
}

pub fn http_port() -> u16 {
    config().server.http_port
}

pub fn game_port() -> u16 {
    config().server.game_port
}

pub fn data_directory() -> &'static PathBuf {
    &config().paths.static_data
}

pub fn excel_data_directory() -> &'static PathBuf {
    &config().paths.excel_data
}

pub fn init_tracing() {
    #[cfg(target_os = "windows")]
    let _ = ansi_term::enable_ansi_support();

    tracing_subscriber::fmt().init();
}

use std::time::{SystemTime, UNIX_EPOCH};

pub fn cur_time_ms_u128() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn time_ms_u64() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
