use crate::error::{GatewayError, Result as AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_PROVIDER_COLLECTION: &str = "默认合集";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub load_balancing: LoadBalancing,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default = "default_provider_collection")]
    pub collection: String,
    pub api_type: ProviderType,
    pub base_url: String,
    pub api_keys: Vec<String>,
    pub models_endpoint: Option<String>,
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    Zhipu,
    Doubao,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancing {
    pub strategy: BalanceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum BalanceStrategy {
    #[default]
    FirstAvailable,
    RoundRobin,
    Random,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub admin_secret: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
            admin_secret: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_database_path")]
    pub database_path: String,
    #[serde(default)]
    pub key_log_strategy: Option<KeyLogStrategy>,
    #[serde(default)]
    pub pg_url: Option<String>,
    #[serde(default)]
    pub pg_schema: Option<String>,
    #[serde(default)]
    pub pg_pool_size: Option<usize>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            database_path: "data/gateway.db".to_string(),
            key_log_strategy: Some(KeyLogStrategy::Masked),
            pg_url: None,
            pg_schema: None,
            pg_pool_size: None,
        }
    }
}

fn default_database_path() -> String {
    "data/gateway.db".to_string()
}

fn default_provider_enabled() -> bool {
    true
}

fn default_provider_collection() -> String {
    DEFAULT_PROVIDER_COLLECTION.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyLogStrategy {
    None,
    Masked,
    Plain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRedirect {
    pub redirects: HashMap<String, String>,
}

impl Settings {
    pub fn load() -> AppResult<Self> {
        let config_path = Self::find_config_file()?;
        let config_content = std::fs::read_to_string(&config_path)?;
        let settings: Settings = toml::from_str(&config_content)?;

        Ok(settings)
    }

    pub fn load_model_redirects() -> AppResult<ModelRedirect> {
        let redirect_path = "redirect.toml";
        if Path::new(redirect_path).exists() {
            let content = std::fs::read_to_string(redirect_path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(ModelRedirect {
                redirects: HashMap::new(),
            })
        }
    }

    fn find_config_file() -> AppResult<String> {
        let possible_names = ["custom-config.toml", "config.toml"];

        for name in &possible_names {
            if Path::new(name).exists() {
                return Ok(name.to_string());
            }
        }

        Err(GatewayError::Config(
            "Configuration file not found. Please create custom-config.toml or config.toml".into(),
        ))
    }
}
