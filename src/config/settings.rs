use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub providers: HashMap<String, Provider>,
    pub load_balancing: LoadBalancing,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub name: String,
    pub api_type: ProviderType,
    pub base_url: String,
    pub api_keys: Vec<String>,
    pub models_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    OpenAI,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancing {
    pub strategy: BalanceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalanceStrategy {
    FirstAvailable,
    RoundRobin,
    Random,
}

impl Default for BalanceStrategy {
    fn default() -> Self {
        Self::FirstAvailable
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub database_path: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            database_path: "data/gateway.db".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRedirect {
    pub redirects: HashMap<String, String>,
}

impl Settings {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = Self::find_config_file()?;
        let config_content = std::fs::read_to_string(&config_path)?;
        let mut settings: Settings = toml::from_str(&config_content)?;

        for (name, provider) in &mut settings.providers {
            provider.name = name.clone();
        }

        Ok(settings)
    }

    pub fn load_model_redirects() -> Result<ModelRedirect, Box<dyn std::error::Error>> {
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

    fn find_config_file() -> Result<String, Box<dyn std::error::Error>> {
        let possible_names = ["custom-config.toml", "config.toml"];

        for name in &possible_names {
            if Path::new(name).exists() {
                return Ok(name.to_string());
            }
        }

        Err("Configuration file not found. Please create custom-config.toml or config.toml".into())
    }
}