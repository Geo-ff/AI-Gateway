use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyRotationStrategy {
    Sequential,
    Random,
    WeightedSequential,
    WeightedRandom,
}

impl Default for KeyRotationStrategy {
    fn default() -> Self {
        Self::WeightedSequential
    }
}

impl KeyRotationStrategy {
    pub fn from_db_value(value: Option<&str>) -> Self {
        match value.unwrap_or_default().to_ascii_lowercase().as_str() {
            "sequential" => Self::Sequential,
            "random" => Self::Random,
            "weighted_random" => Self::WeightedRandom,
            "weighted_sequential" => Self::WeightedSequential,
            _ => Self::default(),
        }
    }

    pub fn as_db_value(&self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Random => "random",
            Self::WeightedSequential => "weighted_sequential",
            Self::WeightedRandom => "weighted_random",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderKeyEntry {
    pub value: String,
    pub active: bool,
    pub weight: u32,
}
