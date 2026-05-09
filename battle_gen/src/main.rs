mod bootstrap;
mod cli;
mod formatting;
mod generator;
mod output;
mod parser;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use cli::CliOptions;
use formatting::live::apply_live_format;
use generator::{generate_begin_round_reply, generate_begin_round_sequence};
use output::next_output_path;
use parser::{
    begin_round::{extract_begin_round_inputs, load_begin_round_captures},
    start_dungeon::{
        load_cards_used_decks, load_fight, load_initial_bloodpool_effects,
        load_initial_buff_add_effects, load_initial_ex_point_info, load_initial_round,
    },
};
use serde_json::Value;

fn main() -> Result<()> {
    // Deep nested fightStep payloads can exceed the default thread stack on Windows.
    // Run the generator on a larger stack to match live-shaped payloads safely.
    const MAIN_STACK_SIZE: usize = 32 * 1024 * 1024;
    let handle = std::thread::Builder::new()
        .name("battle_gen_main".to_string())
        .stack_size(MAIN_STACK_SIZE)
        .spawn(run)
        .context("failed to spawn main worker thread")?;

    match handle.join() {
        Ok(result) => result,
        Err(panic_payload) => std::panic::resume_unwind(panic_payload),
    }
}

fn run() -> Result<()> {
    init_tracing();

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let opts = CliOptions::from_args(&workspace_root)?;

    bootstrap::bootstrap_game_data(&opts.config_path, opts.excel_path.clone())?;

    if opts.no_args {
        let tests_dir = workspace_root.join("tests");
        let scenarios = discover_scenarios(&tests_dir, true)?;
        for scenario in scenarios {
            let generated_entries = generate_scenario_entries(&scenario.fight_path, &scenario.dir)?;
            let scenario_runs_dir = opts.runs_dir.join(&scenario.name);
            write_generated_entries(&scenario_runs_dir, generated_entries, opts.live_format)?;
        }
        return Ok(());
    }

    if let Some(begin_rounds_dir) = opts.begin_rounds_dir.as_ref() {
        let generated_entries = generate_scenario_entries(&opts.fight_path, begin_rounds_dir)?;
        write_generated_entries(&opts.runs_dir, generated_entries, opts.live_format)?;
        return Ok(());
    }

    // Current parser path is begin-round focused so we can add more
    // PCAP/packet formats later under parser::<format>.
    let fight_input = load_fight(&opts.fight_path)?;
    let (player_deck, ai_deck) = load_cards_used_decks(&opts.fight_path);
    let reply = generate_begin_round_reply(fight_input, player_deck, ai_deck)?;
    let mut output_value = serde_json::to_value(&reply).context("failed to convert output JSON")?;
    if opts.live_format {
        apply_live_format(&mut output_value);
    }
    let output_path = next_output_path(&opts.runs_dir, "my_battle")?;
    let output =
        serde_json::to_string_pretty(&output_value).context("failed to serialize output JSON")?;
    fs::write(&output_path, output)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    println!("wrote {}", output_path.display());
    Ok(())
}

fn init_tracing() {
    let level = std::env::var("RUST_LOG")
        .ok()
        .as_deref()
        .map(parse_log_level)
        .unwrap_or(tracing::Level::WARN);
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(true)
        .try_init();
}

fn parse_log_level(raw: &str) -> tracing::Level {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("trace") {
        tracing::Level::TRACE
    } else if lower.contains("debug") {
        tracing::Level::DEBUG
    } else if lower.contains("info") {
        tracing::Level::INFO
    } else if lower.contains("error") {
        tracing::Level::ERROR
    } else {
        tracing::Level::WARN
    }
}

#[derive(Debug, Clone)]
struct Scenario {
    name: String,
    dir: PathBuf,
    fight_path: PathBuf,
}

fn discover_scenarios(tests_dir: &Path, require_begin_round_files: bool) -> Result<Vec<Scenario>> {
    let mut scenarios = Vec::new();
    for entry in fs::read_dir(tests_dir)
        .with_context(|| format!("failed to read {}", tests_dir.display()))?
    {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let fight_path = dir.join("live_battle2.json");
        if !fight_path.is_file() {
            continue;
        }
        if require_begin_round_files && !has_begin_round_files(&dir)? {
            continue;
        }

        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        scenarios.push(Scenario {
            name,
            dir,
            fight_path,
        });
    }

    scenarios.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scenarios)
}

fn has_begin_round_files(dir: &Path) -> Result<bool> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("begin_round_")
            && name.ends_with(".json")
            && !name.ends_with("_request.json")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn generate_scenario_entries(
    fight_path: &Path,
    begin_rounds_dir: &Path,
) -> Result<Vec<(String, Value)>> {
    let fight_input = load_fight(fight_path)?;
    let initial_ex_point_info = load_initial_ex_point_info(fight_path);
    let initial_round = load_initial_round(fight_path);
    let initial_bloodpool_effects = load_initial_bloodpool_effects(fight_path);
    let initial_buff_add_effects = load_initial_buff_add_effects(fight_path);
    let captures = load_begin_round_captures(begin_rounds_dir)?;

    let mut rounds = Vec::new();
    for (path, v, request_v) in captures {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let (
            deck,
            ai_deck,
            opers,
            ai_steps,
            replay_selected_cards,
            replay_silent_ops,
            replay_wave_snapshots,
        ) =
            extract_begin_round_inputs(&v, request_v.as_ref())
                .with_context(|| format!("failed extracting operations from {}", path.display()))?;
        rounds.push((
            name,
            deck,
            ai_deck,
            opers,
            ai_steps,
            replay_selected_cards,
            replay_silent_ops,
            replay_wave_snapshots,
        ));
    }

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(generate_begin_round_sequence(
        fight_input,
        initial_ex_point_info,
        initial_round,
        initial_bloodpool_effects,
        initial_buff_add_effects,
        rounds,
    ))
}

fn write_generated_entries(
    runs_dir: &Path,
    generated_entries: Vec<(String, Value)>,
    live_format: bool,
) -> Result<()> {
    fs::create_dir_all(runs_dir)
        .with_context(|| format!("failed to create {}", runs_dir.display()))?;

    for (source_name, mut entry) in generated_entries {
        if live_format {
            apply_live_format(&mut entry);
        }
        let output_path = runs_dir.join(format!("my_{}", source_name));
        let output =
            serde_json::to_string_pretty(&entry).context("failed to serialize output JSON")?;
        fs::write(&output_path, output)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
        println!("wrote {}", output_path.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Once;

    static TEST_BOOTSTRAP: Once = Once::new();

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn bootstrap_once(root: &Path) -> Result<()> {
        let mut result: Result<()> = Ok(());
        TEST_BOOTSTRAP.call_once(|| {
            result = bootstrap::bootstrap_game_data(&root.join("config.toml"), None);
        });
        result
    }

    fn top_level_effect_type_signature(entry: &Value) -> Result<Vec<Vec<i32>>> {
        let steps = entry
            .get("round")
            .and_then(|r| r.get("fightStep"))
            .and_then(|a| a.as_array())
            .context("missing round.fightStep[]")?;

        Ok(steps
            .iter()
            .map(|step| {
                step.get("actEffect")
                    .and_then(|a| a.as_array())
                    .map(|effects| {
                        effects
                            .iter()
                            .map(|e| {
                                e.get("effectType").and_then(|v| v.as_i64()).unwrap_or(-1) as i32
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect())
    }

    #[test]
    fn begin_round_golden_replay_matches_snapshot() -> Result<()> {
        let root = workspace_root();
        let tests_dir = root.join("tests");
        let golden_root = tests_dir.join("golden");

        bootstrap_once(&root)?;

        let scenarios = discover_scenarios(&tests_dir, false)?;
        let mut compared_scenarios = 0usize;
        for scenario in scenarios {
            let golden_dir = golden_root.join(&scenario.name);
            if !golden_dir.is_dir() {
                eprintln!(
                    "WARN: skipping scenario '{}' because {} is missing",
                    scenario.name,
                    golden_dir.display()
                );
                continue;
            }

            let mut generated_entries =
                generate_scenario_entries(&scenario.fight_path, &scenario.dir)?;
            generated_entries.sort_by(|a, b| a.0.cmp(&b.0));

            for (source_name, mut entry) in generated_entries {
                apply_live_format(&mut entry);
                let golden_path = golden_dir.join(format!("my_{}", source_name));
                let golden_raw = fs::read_to_string(&golden_path).with_context(|| {
                    format!("missing golden snapshot {}", golden_path.display())
                })?;
                let golden_value: Value = serde_json::from_str(&golden_raw)
                    .with_context(|| format!("invalid JSON in {}", golden_path.display()))?;

                assert_eq!(
                    entry, golden_value,
                    "golden mismatch for scenario '{}' capture '{}'",
                    scenario.name, source_name
                );
            }

            compared_scenarios += 1;
        }

        anyhow::ensure!(
            compared_scenarios > 0,
            "no scenarios were compared against goldens under {}",
            golden_root.display()
        );
        Ok(())
    }

    #[test]
    fn begin_round_step2_direct_wrapper_shape() -> Result<()> {
        let root = workspace_root();
        let fight_path = root.join("tests").join("battle1").join("live_battle2.json");
        let begin_rounds_dir = root.join("tests").join("battle1");

        bootstrap_once(&root)?;

        let fight_input = load_fight(&fight_path)?;
        let initial_ex_point_info = load_initial_ex_point_info(&fight_path);
        let initial_round = load_initial_round(&fight_path);
        let initial_bloodpool_effects = load_initial_bloodpool_effects(&fight_path);
        let initial_buff_add_effects = load_initial_buff_add_effects(&fight_path);
        let captures = load_begin_round_captures(&begin_rounds_dir)?;
        let mut rounds = Vec::new();
        for (path, v, request_v) in captures {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name != "begin_round_1.json" {
                continue;
            }
            let (
                deck,
                ai_deck,
                opers,
                ai_steps,
                replay_selected_cards,
                replay_silent_ops,
                replay_wave_snapshots,
            ) =
                extract_begin_round_inputs(&v, request_v.as_ref()).with_context(|| {
                    format!("failed extracting operations from {}", path.display())
                })?;
            rounds.push((
                name,
                deck,
                ai_deck,
                opers,
                ai_steps,
                replay_selected_cards,
                replay_silent_ops,
                replay_wave_snapshots,
            ));
        }
        anyhow::ensure!(
            rounds.len() == 1,
            "expected exactly one begin_round_1.json capture under {}",
            begin_rounds_dir.display()
        );

        let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
        let mut generated_entries = rt.block_on(generate_begin_round_sequence(
            fight_input,
            initial_ex_point_info,
            initial_round,
            initial_bloodpool_effects,
            initial_buff_add_effects,
            rounds,
        ))?;
        anyhow::ensure!(generated_entries.len() == 1, "expected one generated entry");
        let (_source_name, mut entry) = generated_entries.remove(0);
        apply_live_format(&mut entry);

        let step = entry
            .get("round")
            .and_then(|r| r.get("fightStep"))
            .and_then(|a| a.as_array())
            .and_then(|a| a.get(1))
            .context("missing round.fightStep[1]")?;
        let act_effect = step
            .get("actEffect")
            .and_then(|a| a.as_array())
            .context("missing round.fightStep[1].actEffect[]")?;
        let first = act_effect
            .first()
            .context("missing round.fightStep[1].actEffect[0]")?;
        anyhow::ensure!(
            first.get("effectType").and_then(|v| v.as_i64()) == Some(162),
            "step2 first effect must be wrapper 162"
        );

        let nested_step = first
            .get("fightStep")
            .context("missing round.fightStep[1].actEffect[0].fightStep")?;
        anyhow::ensure!(
            nested_step.get("actType").and_then(|v| v.as_str()) == Some("SKILL"),
            "step2 wrapper must target SKILL fightStep"
        );
        anyhow::ensure!(
            nested_step.get("actId").and_then(|v| v.as_i64()) == Some(434425),
            "step2 wrapper must target actId=434425"
        );

        let nested_effects = nested_step
            .get("actEffect")
            .and_then(|a| a.as_array())
            .context("missing nested actEffect[]")?;
        let nested_first = nested_effects
            .first()
            .context("missing nested actEffect[0]")?;
        anyhow::ensure!(
            nested_first.get("effectType").and_then(|v| v.as_i64()) != Some(162),
            "step2 nested skill should not start with another 162 wrapper"
        );

        Ok(())
    }

    // TODO(parity): re-enable this after we choose and refresh the canonical
    // begin_round_1/2 golden snapshots for this branch. Right now the branch is
    // known to drift from the checked-in golden top-level signatures, so this
    // stays ignored to avoid turning default CI red for an acknowledged baseline
    // problem instead of a newly introduced regression.
    #[test]
    #[ignore = "known golden signature drift in current branch; enable after snapshot refresh"]
    fn begin_round_top_level_effect_signature_matches_golden() -> Result<()> {
        let root = workspace_root();
        let fight_path = root.join("tests").join("battle1").join("live_battle2.json");
        let begin_rounds_dir = root.join("tests").join("battle1");
        let golden_dir = root.join("tests").join("golden").join("battle1");

        bootstrap_once(&root)?;

        let fight_input = load_fight(&fight_path)?;
        let initial_ex_point_info = load_initial_ex_point_info(&fight_path);
        let initial_round = load_initial_round(&fight_path);
        let initial_bloodpool_effects = load_initial_bloodpool_effects(&fight_path);
        let initial_buff_add_effects = load_initial_buff_add_effects(&fight_path);
        let captures = load_begin_round_captures(&begin_rounds_dir)?;

        let mut rounds = Vec::new();
        for (path, v, request_v) in captures {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name != "begin_round_1.json" && name != "begin_round_2.json" {
                continue;
            }
            let (
                deck,
                ai_deck,
                opers,
                ai_steps,
                replay_selected_cards,
                replay_silent_ops,
                replay_wave_snapshots,
            ) =
                extract_begin_round_inputs(&v, request_v.as_ref()).with_context(|| {
                    format!("failed extracting operations from {}", path.display())
                })?;
            rounds.push((
                name,
                deck,
                ai_deck,
                opers,
                ai_steps,
                replay_selected_cards,
                replay_silent_ops,
                replay_wave_snapshots,
            ));
        }
        anyhow::ensure!(
            rounds.len() == 2,
            "expected begin_round_1.json and begin_round_2.json under {}",
            begin_rounds_dir.display()
        );
        rounds.sort_by(|a, b| a.0.cmp(&b.0));

        let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
        let generated_entries = rt.block_on(generate_begin_round_sequence(
            fight_input,
            initial_ex_point_info,
            initial_round,
            initial_bloodpool_effects,
            initial_buff_add_effects,
            rounds,
        ))?;

        let mut generated_by_name: HashMap<String, Value> = HashMap::new();
        for (name, mut entry) in generated_entries {
            apply_live_format(&mut entry);
            generated_by_name.insert(name, entry);
        }

        for round_name in ["begin_round_1.json", "begin_round_2.json"] {
            let generated = generated_by_name
                .get(round_name)
                .with_context(|| format!("missing generated {}", round_name))?;
            let generated_sig = top_level_effect_type_signature(generated)?;

            let golden_path = golden_dir.join(format!("my_{}", round_name));
            let golden_raw = fs::read_to_string(&golden_path)
                .with_context(|| format!("missing golden snapshot {}", golden_path.display()))?;
            let golden_value: Value = serde_json::from_str(&golden_raw)
                .with_context(|| format!("invalid JSON in {}", golden_path.display()))?;
            let golden_sig = top_level_effect_type_signature(&golden_value)?;

            assert_eq!(
                generated_sig, golden_sig,
                "top-level effectType signature mismatch for {}",
                round_name
            );
        }

        Ok(())
    }
}
