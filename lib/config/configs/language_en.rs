// Auto-generated from JSON data
// Do not edit manually

use std::collections::HashMap;

pub struct LanguageEnTable {
    by_key: HashMap<String, String>,
}

impl LanguageEnTable {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&json)?;

        let rows: Vec<(String, String)> = if let Some(array) = value.as_array() {
            if array.len() >= 2 && array[1].is_array() {
                serde_json::from_value(array[1].clone())?
            } else {
                serde_json::from_value(value)?
            }
        } else {
            serde_json::from_value(value)?
        };

        let by_key: HashMap<String, String> = rows.into_iter().collect();
        Ok(Self { by_key })
    }

    /// Resolve a `language_xxx` key to its localized text.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(|s| s.as_str())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}
