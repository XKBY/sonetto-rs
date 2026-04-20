pub use prost;

pub mod serde_helpers {
    use serde::Deserialize;
    use serde::de::{self, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum I64OrString {
        I64(i64),
        String(String),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U64OrString {
        U64(u64),
        String(String),
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum I32OrString {
        I32(i32),
        String(String),
    }

    pub fn option_i64_from_number_or_string<'de, D>(
        deserializer: D,
    ) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<I64OrString>::deserialize(deserializer)?;
        match value {
            None => Ok(None),
            Some(I64OrString::I64(v)) => Ok(Some(v)),
            Some(I64OrString::String(v)) => v.parse::<i64>().map(Some).map_err(de::Error::custom),
        }
    }

    pub fn vec_i64_from_number_or_string<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<I64OrString>::deserialize(deserializer)?;
        values
            .into_iter()
            .map(|value| match value {
                I64OrString::I64(v) => Ok(v),
                I64OrString::String(v) => v.parse::<i64>().map_err(de::Error::custom),
            })
            .collect()
    }

    pub fn option_u64_from_number_or_string<'de, D>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<U64OrString>::deserialize(deserializer)?;
        match value {
            None => Ok(None),
            Some(U64OrString::U64(v)) => Ok(Some(v)),
            Some(U64OrString::String(v)) => v.parse::<u64>().map(Some).map_err(de::Error::custom),
        }
    }

    pub fn vec_u64_from_number_or_string<'de, D>(deserializer: D) -> Result<Vec<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<U64OrString>::deserialize(deserializer)?;
        values
            .into_iter()
            .map(|value| match value {
                U64OrString::U64(v) => Ok(v),
                U64OrString::String(v) => v.parse::<u64>().map_err(de::Error::custom),
            })
            .collect()
    }

    pub fn option_enum_i32_from_string_or_number<'de, D, F>(
        deserializer: D,
        parse_name: F,
    ) -> Result<Option<i32>, D::Error>
    where
        D: Deserializer<'de>,
        F: Fn(&str) -> Option<i32>,
    {
        let value = Option::<I32OrString>::deserialize(deserializer)?;
        match value {
            None => Ok(None),
            Some(I32OrString::I32(v)) => Ok(Some(v)),
            Some(I32OrString::String(v)) => {
                let trimmed = v.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                if let Ok(parsed) = trimmed.parse::<i32>() {
                    return Ok(Some(parsed));
                }
                parse_name(trimmed)
                    .map(Some)
                    .ok_or_else(|| de::Error::custom(format!("unknown enum variant: {trimmed}")))
            }
        }
    }
}

include!("../include/_.rs");
