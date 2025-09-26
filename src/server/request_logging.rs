use chrono::{DateTime, Utc};
use crate::logging::RequestLog;
use crate::providers::openai::ChatCompletionResponse;
use crate::server::AppState;

// 记录聊天请求日志（包含响应耗时和 token 使用情况）
pub async fn log_chat_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    model: &str,
    provider_name: &str,
    response: &Result<ChatCompletionResponse, reqwest::Error>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        model: Some(model.to_string()),
        provider: Some(provider_name.to_string()),
        status_code: if response.is_ok() { 200 } else { 500 },
        response_time_ms,
        prompt_tokens: response.as_ref().ok().map(|r| r.usage.prompt_tokens),
        completion_tokens: response.as_ref().ok().map(|r| r.usage.completion_tokens),
        total_tokens: response.as_ref().ok().map(|r| r.usage.total_tokens),
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }
}
