use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub provider: Option<String>,
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

