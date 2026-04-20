use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

pub fn next_output_path(runs_dir: &Path, prefix: &str) -> Result<PathBuf> {
    fs::create_dir_all(runs_dir)
        .with_context(|| format!("failed to create {}", runs_dir.display()))?;

    let mut max_idx = 0_i32;
    for entry in
        fs::read_dir(runs_dir).with_context(|| format!("failed to list {}", runs_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(prefix) || !name.ends_with(".json") {
            continue;
        }
        let suffix = &name[prefix.len()..name.len() - ".json".len()];
        if let Ok(idx) = suffix.parse::<i32>()
            && idx > max_idx
        {
            max_idx = idx;
        }
    }

    Ok(runs_dir.join(format!("{}{}.json", prefix, max_idx + 1)))
}
