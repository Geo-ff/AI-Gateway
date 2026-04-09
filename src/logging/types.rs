use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// 建议统一的请求类型常量（可扩展）
pub const REQ_TYPE_CHAT_ONCE: &str = "chat_once";
pub const REQ_TYPE_CHAT_STREAM: &str = "chat_stream";
pub const REQ_TYPE_CHAT_REPLAY: &str = "chat_replay";
pub const REQ_TYPE_CHAT_COMPARE: &str = "chat_compare";
pub const REQ_TYPE_RECHARGE: &str = "recharge";
pub const REQ_TYPE_MODELS_LIST: &str = "models_list";
pub const REQ_TYPE_PROVIDER_MODELS_LIST: &str = "provider_models_list";
pub const REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST: &str = "provider_models_baseurl_list";
pub const REQ_TYPE_PROVIDER_KEY_ADD: &str = "provider_key_add";
pub const REQ_TYPE_PROVIDER_KEY_DELETE: &str = "provider_key_delete";
pub const REQ_TYPE_PROVIDER_KEY_LIST: &str = "provider_key_list";
pub const REQ_TYPE_PROVIDER_KEY_TOGGLE: &str = "provider_key_toggle";
pub const REQ_TYPE_PROVIDER_KEY_CONFIG_GET: &str = "provider_key_config_get";
pub const REQ_TYPE_PROVIDER_KEY_CONFIG_SET: &str = "provider_key_config_set";
pub const REQ_TYPE_PROVIDER_KEY_WEIGHT_SET: &str = "provider_key_weight_set";
pub const REQ_TYPE_PROVIDER_CACHE_UPDATE: &str = "provider_models_cache_update";
pub const REQ_TYPE_PROVIDER_CACHE_DELETE: &str = "provider_models_cache_delete";
pub const REQ_TYPE_PROVIDER_CREATE: &str = "provider_create";
pub const REQ_TYPE_PROVIDER_UPDATE: &str = "provider_update";
pub const REQ_TYPE_PROVIDER_DELETE: &str = "provider_delete";
pub const REQ_TYPE_PROVIDER_GET: &str = "provider_get";
pub const REQ_TYPE_PROVIDER_LIST: &str = "provider_list";
pub const REQ_TYPE_PROVIDER_ENABLED_SET: &str = "provider_enabled_set";
pub const REQ_TYPE_PROVIDER_FAVORITE_SET: &str = "provider_favorite_set";
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST: &str = "provider_model_redirects_list";
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET: &str = "provider_model_redirects_set";
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE: &str = "provider_model_redirects_delete";
pub const REQ_TYPE_PROVIDER_MODEL_TEST: &str = "provider_model_test";

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub request_type: String,
    /// 用户原始请求传入的模型名（重定向前）
    pub requested_model: Option<String>,
    /// 实际上游调用使用的模型名（重定向后）
    pub effective_model: Option<String>,
    /// 计费/价格计算使用的模型名（历史字段；可能与 effective_model 不同）
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub client_token: Option<String>,
    pub user_id: Option<String>,
    // 本次请求消耗的金额；仅在有价格与 usage 可用时计算
    pub amount_spent: Option<f64>,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogDetailRecord {
    pub request_log_id: i64,
    pub request_payload_snapshot: Option<String>,
    pub response_preview: Option<String>,
    pub upstream_status: Option<i64>,
    pub fallback_triggered: Option<bool>,
    pub fallback_reason: Option<String>,
    pub selected_provider: Option<String>,
    pub selected_key_id: Option<String>,
    pub first_token_latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCompareRun {
    pub id: String,
    pub user_id: String,
    pub source_request_id: i64,
    pub created_at: DateTime<Utc>,
    pub result_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRequestLabSource {
    pub user_id: String,
    pub source_request_id: i64,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub source_timestamp: DateTime<Utc>,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRequestLabSnapshot {
    pub id: String,
    pub user_id: String,
    pub source_request_id: i64,
    pub compare_run_id: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub snapshot_json: String,
    pub source_requested_model: Option<String>,
    pub source_effective_model: Option<String>,
    pub models: Vec<String>,
    pub success_count: u32,
    pub failure_count: u32,
}

fn default_preserve_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestLabExperimentConfig {
    #[serde(default)]
    pub temperature: Option<serde_json::Value>,
    #[serde(default)]
    pub top_p: Option<serde_json::Value>,
    #[serde(default)]
    pub max_tokens: Option<serde_json::Value>,
    #[serde(default)]
    pub presence_penalty: Option<serde_json::Value>,
    #[serde(default)]
    pub frequency_penalty: Option<serde_json::Value>,
    #[serde(default = "default_preserve_true")]
    pub preserve_system_prompt: bool,
    #[serde(default = "default_preserve_true")]
    pub preserve_message_structure: bool,
}

impl Default for RequestLabExperimentConfig {
    fn default() -> Self {
        Self {
            temperature: None,
            top_p: None,
            max_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            preserve_system_prompt: true,
            preserve_message_structure: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRequestLabTemplate {
    pub id: String,
    pub user_id: String,
    pub scope: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub source_request_id: i64,
    pub compare_models: Vec<String>,
    pub experiment_config: RequestLabExperimentConfig,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ProviderKeyStatsAgg {
    pub api_key: String,
    pub total_requests: u64,
    pub success_count: u64,
    pub failure_count: u64,
}

#[derive(Debug, Clone)]
pub struct CachedModel {
    pub id: String,
    pub provider: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
    pub cached_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ProviderOpLog {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    pub provider: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelPriceSource {
    #[default]
    Manual,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelPriceStatus {
    #[default]
    Active,
    Missing,
    Stale,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPriceRecord {
    pub provider: String,
    pub model: String,
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
    pub currency: Option<String>,
    pub model_type: Option<String>,
    pub source: ModelPriceSource,
    pub status: ModelPriceStatus,
    pub synced_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPriceUpsert {
    pub provider: String,
    pub model: String,
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
    pub currency: Option<String>,
    pub model_type: Option<String>,
    pub source: ModelPriceSource,
    pub status: ModelPriceStatus,
    pub synced_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl ModelPriceUpsert {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn manual(
        provider: impl Into<String>,
        model: impl Into<String>,
        prompt_price_per_million: f64,
        completion_price_per_million: f64,
        currency: Option<String>,
        model_type: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            prompt_price_per_million,
            completion_price_per_million,
            currency,
            model_type,
            source: ModelPriceSource::Manual,
            status: ModelPriceStatus::Active,
            synced_at: None,
            expires_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelPriceSource, ModelPriceStatus};

    #[test]
    fn model_price_enums_serialize_and_deserialize() {
        assert_eq!(
            serde_json::to_string(&ModelPriceSource::Manual).unwrap(),
            "\"manual\""
        );
        assert_eq!(
            serde_json::to_string(&ModelPriceStatus::Stale).unwrap(),
            "\"stale\""
        );
        assert_eq!(
            serde_json::from_str::<ModelPriceSource>("\"auto\"").unwrap(),
            ModelPriceSource::Auto
        );
        assert_eq!(
            serde_json::from_str::<ModelPriceStatus>("\"missing\"").unwrap(),
            ModelPriceStatus::Missing
        );
    }
}
