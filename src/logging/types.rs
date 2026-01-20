use chrono::{DateTime, Utc};

// 建议统一的请求类型常量（可扩展）
pub const REQ_TYPE_CHAT_ONCE: &str = "chat_once";
pub const REQ_TYPE_CHAT_STREAM: &str = "chat_stream";
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
