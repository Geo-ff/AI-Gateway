use chrono::{DateTime, Utc};
use crate::logging::RequestLog;
use crate::logging::types::{REQ_TYPE_CHAT_ONCE};
use crate::config::settings::{KeyLogStrategy, LoggingConfig};
use crate::providers::openai::types::RawAndTypedChatCompletion;
use crate::error::GatewayError;
use crate::server::AppState;

// 记录聊天请求日志（包含响应耗时和 token 使用情况）
pub async fn log_chat_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    model: &str,
    provider_name: &str,
    api_key_raw: &str,
    response: &Result<RawAndTypedChatCompletion, GatewayError>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    let api_key = api_key_hint(&app_state.config.logging, api_key_raw);

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_ONCE.to_string(),
        model: Some(model.to_string()),
        provider: Some(provider_name.to_string()),
        api_key,
        status_code: if response.is_ok() { 200 } else { 500 },
        response_time_ms,
        prompt_tokens: response.as_ref().ok().and_then(|r| r.typed.usage.as_ref().map(|u| u.prompt_tokens)),
        completion_tokens: response.as_ref().ok().and_then(|r| r.typed.usage.as_ref().map(|u| u.completion_tokens)),
        total_tokens: response.as_ref().ok().and_then(|r| r.typed.usage.as_ref().map(|u| u.total_tokens)),
        cached_tokens: response.as_ref().ok().and_then(|r| r.typed.usage.as_ref().and_then(|u| u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens))),
        reasoning_tokens: response.as_ref().ok().and_then(|r| r.typed.usage.as_ref().and_then(|u| u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens))),
        error_message: response.as_ref().err().map(|e| e.to_string()),
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }
}

fn api_key_hint(cfg: &LoggingConfig, key: &str) -> Option<String> {
    match cfg.key_log_strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 { return "****".to_string(); }
    let (start, end) = (&key[..4], &key[key.len()-4..]);
    format!("{}****{}", start, end)
}

// 记录普通请求（不含 tokens）
pub async fn log_simple_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    method: &str,
    path: &str,
    request_type: &str,
    model: Option<String>,
    provider: Option<String>,
    status_code: u16,
    error_message: Option<String>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: method.to_string(),
        path: path.to_string(),
        request_type: request_type.to_string(),
        model,
        provider,
        api_key: None,
        status_code,
        response_time_ms,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
        error_message,
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }
}
