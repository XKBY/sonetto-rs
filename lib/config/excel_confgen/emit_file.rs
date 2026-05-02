use serde_json::Value;

use crate::excel_confgen::{
    emit_struct::emit_struct, emit_table::emit_table_store, rust_ident::snake_to_pascal,
    schema_infer::analyze_field_types,
};

/// `language_en.json` and friends store rows as `[String, String]` 2-tuples
/// (`["language_xxx", "Localized text"]`) rather than the usual object
/// shape. The auto-inferred struct path produces an empty struct with no
/// `by_id` index, which is useless. Detect the kv-string shape and emit a
/// specialized `HashMap`-backed module instead.
fn is_string_kv_table(records: &[Value]) -> bool {
    !records.is_empty()
        && records.iter().all(|r| match r {
            Value::Array(a) => a.len() == 2 && a[0].is_string() && a[1].is_string(),
            _ => false,
        })
}

fn emit_string_kv_module(table: &str) -> String {
    let pascal = snake_to_pascal(table);
    format!(
        r#"// Auto-generated from JSON data
// Do not edit manually

use std::collections::HashMap;

pub struct {pascal}Table {{
    by_key: HashMap<String, String>,
}}

impl {pascal}Table {{
    pub fn load(path: &str) -> anyhow::Result<Self> {{
        let json = std::fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&json)?;

        let rows: Vec<(String, String)> = if let Some(array) = value.as_array() {{
            if array.len() >= 2 && array[1].is_array() {{
                serde_json::from_value(array[1].clone())?
            }} else {{
                serde_json::from_value(value)?
            }}
        }} else {{
            serde_json::from_value(value)?
        }};

        let by_key: HashMap<String, String> = rows.into_iter().collect();
        Ok(Self {{ by_key }})
    }}

    /// Resolve a `language_xxx` key to its localized text.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&str> {{
        self.by_key.get(key).map(|s| s.as_str())
    }}

    #[inline]
    pub fn len(&self) -> usize {{
        self.by_key.len()
    }}

    #[inline]
    pub fn is_empty(&self) -> bool {{
        self.by_key.is_empty()
    }}
}}
"#,
        pascal = pascal
    )
}

pub fn emit_table_file(table: &str, records: &[Value]) -> String {
    if is_string_kv_table(records) {
        return emit_string_kv_module(table);
    }

    let field_types = analyze_field_types(records);

    [
        "// Auto-generated from JSON data",
        "// Do not edit manually",
        "",
        "use serde::{Deserialize, Serialize};",
        "",
        &emit_struct(table, records),
        &emit_table_store(table, &field_types, records),
    ]
    .join("\n")
}
