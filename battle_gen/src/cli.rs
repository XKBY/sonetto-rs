use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub no_args: bool,
    pub fight_path: PathBuf,
    pub begin_rounds_dir: Option<PathBuf>,
    pub runs_dir: PathBuf,
    pub config_path: PathBuf,
    pub excel_path: Option<PathBuf>,
    pub live_format: bool,
}

impl CliOptions {
    pub fn from_args(workspace_root: &Path) -> Result<Self> {
        let raw_args: Vec<String> = std::env::args().skip(1).collect();
        let no_args = raw_args.is_empty();

        let mut fight_path = workspace_root.join("tests/fight.json");
        let mut begin_rounds_dir: Option<PathBuf> = None;
        let mut runs_dir = workspace_root.join("tests").join("runs");
        let mut config_path = workspace_root.join("config.toml");
        let mut excel_path: Option<PathBuf> = None;
        let mut live_format = true;

        let mut args = raw_args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--config" => {
                    let Some(v) = args.next() else {
                        bail!("--config requires a file path");
                    };
                    config_path = PathBuf::from(v);
                }
                "--fight" => {
                    let Some(v) = args.next() else {
                        bail!("--fight requires a file path");
                    };
                    fight_path = PathBuf::from(v);
                }
                "--begin-rounds-dir" => {
                    let Some(v) = args.next() else {
                        bail!("--begin-rounds-dir requires a directory path");
                    };
                    begin_rounds_dir = Some(PathBuf::from(v));
                }
                "--excel" => {
                    let Some(v) = args.next() else {
                        bail!("--excel requires a directory path");
                    };
                    excel_path = Some(PathBuf::from(v));
                }
                "--runs" => {
                    let Some(v) = args.next() else {
                        bail!("--runs requires a directory path");
                    };
                    runs_dir = PathBuf::from(v);
                }
                "--live-format" => {
                    live_format = true;
                }
                "--raw-format" => {
                    live_format = false;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    bail!("unknown argument: {}", other);
                }
            }
        }

        Ok(Self {
            no_args,
            fight_path,
            begin_rounds_dir,
            runs_dir,
            config_path,
            excel_path,
            live_format,
        })
    }
}

fn print_help() {
    println!("battle_gen usage:");
    println!(
        "  cargo run -p battle_gen -- [--fight PATH] [--begin-rounds-dir DIR] [--runs DIR] [--live-format]"
    );
    println!("defaults:");
    println!("  with no args: scans tests/<scenario>/ and replays each scenario");
    println!("  --config config.toml");
    println!("  --excel  (optional) excel data directory override");
    println!("  --fight tests/fight.json");
    println!("           also used as cards source (round.teamACards1 + round.aiUseCards)");
    println!("  --begin-rounds-dir (single-scenario replay mode)");
    println!("  --runs  tests/runs");
    println!("  --live-format reorder JSON keys to match client-style output (default)");
    println!("  --raw-format disable live formatting and keep raw serde/proto order");
}
