use crate::error::{GatewayError, Result as AppResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

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
    #[serde(default)]
    pub api_type_raw: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderType {
    OpenAI,
    AzureOpenAI,
    Anthropic,
    AwsClaude,
    GoogleGemini,
    VertexAI,
    Cohere,
    Cloudflare,
    Perplexity,
    Mistral,
    DeepSeek,
    SiliconCloud,
    Moonshot,
    Zhipu,
    AlibabaQwen,
    Custom,
    XAI,
    Doubao,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMode {
    Bearer,
    XApiKey,
    ApiKey,
    SigV4,
    OAuth,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocolFamily {
    OpenAI,
    Anthropic,
    Zhipu,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub auth_mode: ProviderAuthMode,
    pub supports_auto_model_discovery: bool,
    pub supports_models_endpoint: bool,
    pub requires_models_endpoint: bool,
    pub test_connection_family: ProviderProtocolFamily,
    pub openai_compatible: bool,
}

impl ProviderType {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderType::OpenAI => "openai",
            ProviderType::AzureOpenAI => "azure_openai",
            ProviderType::Anthropic => "anthropic",
            ProviderType::AwsClaude => "aws_claude",
            ProviderType::GoogleGemini => "google_gemini",
            ProviderType::VertexAI => "vertex_ai",
            ProviderType::Cohere => "cohere",
            ProviderType::Cloudflare => "cloudflare",
            ProviderType::Perplexity => "perplexity",
            ProviderType::Mistral => "mistral",
            ProviderType::DeepSeek => "deepseek",
            ProviderType::SiliconCloud => "siliconcloud",
            ProviderType::Moonshot => "moonshot",
            ProviderType::Zhipu => "zhipu",
            ProviderType::AlibabaQwen => "alibaba_qwen",
            ProviderType::Custom => "custom",
            ProviderType::XAI => "xai",
            ProviderType::Doubao => "doubao",
        }
    }

    pub fn from_storage_with_raw(raw: &str) -> (Self, Option<String>) {
        let trimmed = raw.trim();
        match Self::from_str(trimmed) {
            Ok(provider_type) => {
                let raw_out =
                    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(provider_type.as_str()) {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                (provider_type, raw_out)
            }
            Err(_) => {
                tracing::warn!(
                    raw_api_type = trimmed,
                    "Unknown provider type from storage; falling back to custom openai-compatible semantics"
                );
                (ProviderType::Custom, Some(trimmed.to_string()))
            }
        }
    }

    pub fn auth_mode(self) -> ProviderAuthMode {
        match self {
            ProviderType::Anthropic => ProviderAuthMode::XApiKey,
            ProviderType::AzureOpenAI | ProviderType::GoogleGemini => ProviderAuthMode::ApiKey,
            ProviderType::AwsClaude => ProviderAuthMode::SigV4,
            ProviderType::VertexAI => ProviderAuthMode::OAuth,
            ProviderType::OpenAI
            | ProviderType::Cohere
            | ProviderType::Cloudflare
            | ProviderType::Perplexity
            | ProviderType::Mistral
            | ProviderType::DeepSeek
            | ProviderType::SiliconCloud
            | ProviderType::Moonshot
            | ProviderType::Zhipu
            | ProviderType::AlibabaQwen
            | ProviderType::Custom
            | ProviderType::XAI
            | ProviderType::Doubao => ProviderAuthMode::Bearer,
        }
    }

    pub fn protocol_family(self) -> ProviderProtocolFamily {
        match self {
            ProviderType::Anthropic => ProviderProtocolFamily::Anthropic,
            ProviderType::Zhipu => ProviderProtocolFamily::Zhipu,
            ProviderType::AwsClaude
            | ProviderType::AzureOpenAI
            | ProviderType::GoogleGemini
            | ProviderType::VertexAI
            | ProviderType::Cohere => ProviderProtocolFamily::Unsupported,
            ProviderType::OpenAI
            | ProviderType::Cloudflare
            | ProviderType::Perplexity
            | ProviderType::Mistral
            | ProviderType::DeepSeek
            | ProviderType::SiliconCloud
            | ProviderType::Moonshot
            | ProviderType::AlibabaQwen
            | ProviderType::Custom
            | ProviderType::XAI
            | ProviderType::Doubao => ProviderProtocolFamily::OpenAI,
        }
    }

    // Keep these capability semantics aligned with `captok/src/features/channels/data/provider-registry.ts`.
    // When adding a provider here, update the frontend registry in the same change.
    pub fn capabilities(self) -> ProviderCapabilities {
        match self {
            ProviderType::Anthropic => ProviderCapabilities {
                auth_mode: self.auth_mode(),
                supports_auto_model_discovery: false,
                supports_models_endpoint: true,
                requires_models_endpoint: true,
                test_connection_family: ProviderProtocolFamily::Anthropic,
                openai_compatible: false,
            },
            ProviderType::Zhipu => ProviderCapabilities {
                auth_mode: self.auth_mode(),
                supports_auto_model_discovery: false,
                supports_models_endpoint: true,
                requires_models_endpoint: true,
                test_connection_family: ProviderProtocolFamily::Zhipu,
                openai_compatible: false,
            },
            ProviderType::AwsClaude => ProviderCapabilities {
                auth_mode: self.auth_mode(),
                supports_auto_model_discovery: false,
                supports_models_endpoint: false,
                requires_models_endpoint: false,
                test_connection_family: ProviderProtocolFamily::Unsupported,
                openai_compatible: false,
            },
            ProviderType::AzureOpenAI
            | ProviderType::GoogleGemini
            | ProviderType::VertexAI
            | ProviderType::Cohere => ProviderCapabilities {
                auth_mode: self.auth_mode(),
                supports_auto_model_discovery: false,
                supports_models_endpoint: false,
                requires_models_endpoint: false,
                test_connection_family: ProviderProtocolFamily::Unsupported,
                openai_compatible: false,
            },
            ProviderType::OpenAI
            | ProviderType::Cloudflare
            | ProviderType::Perplexity
            | ProviderType::Mistral
            | ProviderType::DeepSeek
            | ProviderType::SiliconCloud
            | ProviderType::Moonshot
            | ProviderType::AlibabaQwen
            // `custom` is intentionally scoped to "custom OpenAI-compatible endpoint" for this stage.
            | ProviderType::Custom
            | ProviderType::XAI
            | ProviderType::Doubao => ProviderCapabilities {
                auth_mode: self.auth_mode(),
                supports_auto_model_discovery: true,
                supports_models_endpoint: true,
                requires_models_endpoint: false,
                test_connection_family: ProviderProtocolFamily::OpenAI,
                openai_compatible: true,
            },
        }
    }

    pub fn supports_test_connection(self) -> bool {
        self.capabilities().test_connection_family != ProviderProtocolFamily::Unsupported
    }
}

impl FromStr for ProviderType {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_lowercase().as_str() {
            "openai" | "open-ai" | "open_ai" => Ok(ProviderType::OpenAI),
            "azure_openai" | "azure-openai" | "azure" => Ok(ProviderType::AzureOpenAI),
            "anthropic" => Ok(ProviderType::Anthropic),
            "aws_claude" | "aws-claude" | "bedrock_claude" => Ok(ProviderType::AwsClaude),
            "google_gemini" | "google-gemini" | "google" | "gemini" => {
                Ok(ProviderType::GoogleGemini)
            }
            "vertex_ai" | "vertex-ai" | "vertex" => Ok(ProviderType::VertexAI),
            "cohere" => Ok(ProviderType::Cohere),
            "cloudflare" => Ok(ProviderType::Cloudflare),
            "perplexity" => Ok(ProviderType::Perplexity),
            "mistral" => Ok(ProviderType::Mistral),
            "deepseek" => Ok(ProviderType::DeepSeek),
            "siliconcloud" | "silicon_cloud" | "silicon-cloud" => Ok(ProviderType::SiliconCloud),
            "moonshot" => Ok(ProviderType::Moonshot),
            "zhipu" => Ok(ProviderType::Zhipu),
            "alibaba_qwen" | "alibaba-qwen" | "alibaba" | "qwen" => Ok(ProviderType::AlibabaQwen),
            "custom" => Ok(ProviderType::Custom),
            "xai" | "x_ai" | "x-ai" => Ok(ProviderType::XAI),
            "doubao" => Ok(ProviderType::Doubao),
            other => {
                tracing::warn!(
                    raw_api_type = other,
                    "Unknown provider type from request parsing"
                );
                Err(format!("unsupported provider type: {other}"))
            }
        }
    }
}

impl Serialize for ProviderType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        ProviderType::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderAuthMode, ProviderType};

    #[test]
    fn provider_auth_modes_are_explicit_for_high_priority_types() {
        assert_eq!(
            ProviderType::OpenAI.capabilities().auth_mode,
            ProviderAuthMode::Bearer
        );
        assert_eq!(
            ProviderType::AzureOpenAI.capabilities().auth_mode,
            ProviderAuthMode::ApiKey
        );
        assert_eq!(
            ProviderType::Anthropic.capabilities().auth_mode,
            ProviderAuthMode::XApiKey
        );
        assert_eq!(
            ProviderType::AwsClaude.capabilities().auth_mode,
            ProviderAuthMode::SigV4
        );
        assert_eq!(
            ProviderType::GoogleGemini.capabilities().auth_mode,
            ProviderAuthMode::ApiKey
        );
        assert_eq!(
            ProviderType::VertexAI.capabilities().auth_mode,
            ProviderAuthMode::OAuth
        );
    }

    #[test]
    fn unknown_storage_types_keep_raw_information() {
        let (provider_type, raw) = ProviderType::from_storage_with_raw("future_vendor");
        assert_eq!(provider_type, ProviderType::Custom);
        assert_eq!(raw.as_deref(), Some("future_vendor"));
    }
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
