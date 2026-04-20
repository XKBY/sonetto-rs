use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path};
use walkdir::WalkDir;

use crate::excel_confgen::{
    emit_file::emit_table_file, emit_root::emit_root_module, tables::FILTER_TABLES,
};

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if let Ok(existing) = fs::read_to_string(path)
        && existing == content
    {
        return Ok(());
    }
    fs::write(path, content).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn generate_rust_modules(json_dir: &str, output_dir: &str) -> Result<()> {
    let output = Path::new(output_dir);
    fs::create_dir_all(output).with_context(|| format!("create_dir_all {}", output.display()))?;

    let filter: HashSet<&str> = FILTER_TABLES.iter().copied().collect();
    let mut tables = Vec::new();

    for entry in WalkDir::new(json_dir).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let raw = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid filename"))?;

        if !filter.contains(raw) {
            continue;
        }

        let snake = raw.to_string();
        tables.push(snake.clone());

        let json = fs::read_to_string(entry.path())
            .with_context(|| format!("read {}", entry.path().display()))?;
        let data: Value = serde_json::from_str(&json)?;

        let (table, records) = match data {
            Value::Array(ref a) if a.len() == 2 => (
                a[0].as_str().unwrap_or(raw).to_string(),
                a[1].as_array().cloned().unwrap_or_default(),
            ),
            Value::Array(a) => (raw.to_string(), a),
            _ => (raw.to_string(), vec![data]),
        };

        let out_file = output.join(format!("{}.rs", snake));
        let content = emit_table_file(&table, &records);
        write_if_changed(&out_file, &content)?;
    }

    tables.sort();
    let mod_file = output.join("mod.rs");
    let mod_content = emit_root_module(&tables);
    write_if_changed(&mod_file, &mod_content)?;
    Ok(())
}
