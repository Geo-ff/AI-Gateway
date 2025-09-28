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
    client_token: Option<String>,
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
        client_token,
        amount_spent: None,
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
    client_token: Option<String>,
    usage: Option<Usage>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let (prompt, completion, total, cached, reasoning) = usage
        .as_ref()
        .map(|u| (
            Some(u.prompt_tokens),
            Some(u.completion_tokens),
            Some(u.total_tokens),
            u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
            u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
        ))
        .unwrap_or((None, None, None, None, None));
    // Compute amount_spent if possible (non-admin tokens only)
    let amount_spent = if let (Some(u), Some(tok)) = (usage.as_ref(), client_token.as_deref()) {
        if tok == "admin_token" { None } else {
            match app_state.log_store.get_model_price(&provider, &model).await {
                Ok(Some((p_pm, c_pm, _))) => {
                    let p = u.prompt_tokens as f64 * p_pm / 1_000_000.0;
                    let c = u.completion_tokens as f64 * c_pm / 1_000_000.0;
                    Some(p + c)
                }
                _ => None,
            }
        }
    } else { None };

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
        model: Some(model),
        provider: Some(provider),
        api_key,
        client_token: client_token.clone(),
        amount_spent,
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

    // 增量更新 admin_tokens：金额与 tokens（仅非管理员令牌）
    if let Some(tok) = client_token.as_deref().filter(|t| *t != "admin_token") {
        if let Some(delta) = amount_spent {
            if let Err(e) = app_state.token_store.add_amount_spent(tok, delta).await { tracing::warn!("Failed to update token spent: {}", e); }
        }
        if let Some(u) = usage.as_ref() {
            let prompt = u.prompt_tokens as i64;
            let completion = u.completion_tokens as i64;
            let total = u.total_tokens as i64;
            if let Err(e) = app_state.token_store.add_usage_spent(tok, prompt, completion, total).await { tracing::warn!("Failed to update token tokens: {}", e); }
        }
    }

    // Auto-disable token when exceeding budget (streaming)
    if let Some(tok) = client_token.as_deref() {
        if let Ok(Some(t)) = app_state.token_store.get_token(tok).await {
            if let Some(max_amount) = t.max_amount {
                if t.amount_spent > max_amount { let _ = app_state.token_store.set_enabled(tok, false).await; }
            }
            if let Some(max_tokens) = t.max_tokens {
                if t.total_tokens_spent > max_tokens { let _ = app_state.token_store.set_enabled(tok, false).await; }
            }
        }
    }
}
