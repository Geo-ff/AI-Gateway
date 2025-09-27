use chrono::{DateTime, Utc};

// 建议统一的请求类型常量（可扩展）
pub const REQ_TYPE_CHAT_ONCE: &str = "chat_once";
pub const REQ_TYPE_CHAT_STREAM: &str = "chat_stream";
pub const REQ_TYPE_MODELS_LIST: &str = "models_list";
pub const REQ_TYPE_PROVIDER_MODELS_LIST: &str = "provider_models_list";
pub const REQ_TYPE_PROVIDER_KEY_ADD: &str = "provider_key_add";
pub const REQ_TYPE_PROVIDER_KEY_DELETE: &str = "provider_key_delete";
pub const REQ_TYPE_PROVIDER_CACHE_UPDATE: &str = "provider_models_cache_update";
pub const REQ_TYPE_PROVIDER_CACHE_DELETE: &str = "provider_models_cache_delete";

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
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
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
