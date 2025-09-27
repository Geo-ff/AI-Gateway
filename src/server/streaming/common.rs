use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_CHAT_STREAM;
use crate::server::AppState;
use crate::providers::openai::Usage;

// 统一的流式错误日志记录函数（KISS/DRY）
pub(super) async fn log_stream_error(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model: String,
    provider: String,
    api_key: Option<String>,
    error_message: String,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
        model: Some(model),
        provider: Some(provider),
        api_key,
        status_code: 500,
        response_time_ms,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
        error_message: Some(error_message),
    };
    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log streaming error: {}", e);
    }
}

// 统一的流式成功日志记录函数
pub(super) async fn log_stream_success(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model: String,
    provider: String,
    api_key: Option<String>,
    usage: Option<Usage>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let (prompt, completion, total, cached, reasoning) = usage
        .map(|u| (
            Some(u.prompt_tokens),
            Some(u.completion_tokens),
            Some(u.total_tokens),
            u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
            u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
        ))
        .unwrap_or((None, None, None, None, None));
    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
        model: Some(model),
        provider: Some(provider),
        api_key,
        status_code: 200,
        response_time_ms,
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cached_tokens: cached,
        reasoning_tokens: reasoning,
        error_message: None,
    };
    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log streaming request: {}", e);
    }
}
