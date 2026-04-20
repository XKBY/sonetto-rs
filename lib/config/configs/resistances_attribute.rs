// Auto-generated from JSON data
// Do not edit manually

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResistancesAttribute {
    #[serde(rename = "cantGetExskill")]
    pub cant_get_exskill: i32,
    pub charm: i32,
    #[serde(rename = "controlResilience")]
    pub control_resilience: i32,
    #[serde(rename = "delExPoint")]
    pub del_ex_point: i32,
    #[serde(rename = "delExPointResilience")]
    pub del_ex_point_resilience: i32,
    pub disarm: i32,
    pub dizzy: i32,
    pub forbid: i32,
    pub frozen: i32,
    pub id: i32,
    pub petrified: i32,
    pub seal: i32,
    pub sleep: i32,
    #[serde(rename = "stressUp")]
    pub stress_up: i32,
    #[serde(rename = "stressUpResilience")]
    pub stress_up_resilience: i32,
}
use std::collections::HashMap;

pub struct ResistancesAttributeTable {
    records: Vec<ResistancesAttribute>,
    by_id: HashMap<i32, usize>,
}

impl ResistancesAttributeTable {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&json)?;

        let records: Vec<ResistancesAttribute> = if let Some(array) = value.as_array() {
            if array.len() >= 2 && array[1].is_array() {
                serde_json::from_value(array[1].clone())?
            } else {
                serde_json::from_value(value)?
            }
        } else {
            serde_json::from_value(value)?
        };

        let mut by_id = HashMap::with_capacity(records.len());

        for (idx, record) in records.iter().enumerate() {
            by_id.insert(record.id, idx);
        }

        Ok(Self {
            records,
            by_id,
        })
    }

    #[inline]
    pub fn get(&self, id: i32) -> Option<&ResistancesAttribute> {
        self.by_id.get(&id).map(|&i| &self.records[i])
    }

    #[inline]
    pub fn all(&self) -> &[ResistancesAttribute] {
        &self.records
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, ResistancesAttribute> {
        self.records.iter()
    }

    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }
}