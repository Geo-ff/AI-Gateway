use chrono::{DateTime, Utc};

// 建议统一的请求类型常量（可扩展）
pub const REQ_TYPE_CHAT_ONCE: &str = "chat_once";
pub const REQ_TYPE_CHAT_STREAM: &str = "chat_stream";
pub const REQ_TYPE_MODELS_LIST: &str = "models_list";
pub const REQ_TYPE_PROVIDER_MODELS_LIST: &str = "provider_models_list";
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
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST: &str = "provider_model_redirects_list";
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET: &str = "provider_model_redirects_set";
pub const REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE: &str = "provider_model_redirects_delete";

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub request_type: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub client_token: Option<String>,
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
