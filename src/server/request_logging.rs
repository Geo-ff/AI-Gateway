use crate::error::GatewayError;
use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_CHAT_ONCE;
use crate::providers::openai::types::RawAndTypedChatCompletion;
use crate::server::AppState;
use crate::server::util::api_key_hint;
use chrono::{DateTime, Utc};

// 记录聊天请求日志（包含响应耗时和 token 使用情况）
pub async fn log_chat_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    model: &str,
    provider_name: &str,
    api_key_raw: &str,
    client_token: Option<&str>,
    response: &Result<RawAndTypedChatCompletion, GatewayError>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    let api_key = api_key_hint(&app_state.config.logging, api_key_raw);

    // 计算本次消耗金额（仅当有价格与 usage 可用，且非管理员“身份令牌”）
    let amount_spent: Option<f64> = match response {
        Ok(dual) => {
            let usage = dual.typed.usage.as_ref();
            if let (Some(u), Some(tok)) = (usage, client_token) {
                if tok == "admin_token" {
                    None
                } else {
                    match app_state
                        .log_store
                        .get_model_price(provider_name, model)
                        .await
                    {
                        Ok(Some((p_pm, c_pm, _))) => {
                            let p = u.prompt_tokens as f64 * p_pm / 1_000_000.0;
                            let c = u.completion_tokens as f64 * c_pm / 1_000_000.0;
                            Some(p + c)
                        }
                        _ => None,
                    }
                }
            } else {
                None
            }
        }
        Err(_) => None,
    };

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_ONCE.to_string(),
        model: Some(model.to_string()),
        provider: Some(provider_name.to_string()),
        api_key,
        client_token: client_token.map(|s| s.to_string()),
        amount_spent,
        status_code: if response.is_ok() { 200 } else { 500 },
        response_time_ms,
        prompt_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.prompt_tokens)),
        completion_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.completion_tokens)),
        total_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.total_tokens)),
        cached_tokens: response.as_ref().ok().and_then(|r| {
            r.typed.usage.as_ref().and_then(|u| {
                u.prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens)
            })
        }),
        reasoning_tokens: response.as_ref().ok().and_then(|r| {
            r.typed.usage.as_ref().and_then(|u| {
                u.completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens)
            })
        }),
        error_message: response.as_ref().err().map(|e| e.to_string()),
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }

    // 增量更新 admin_tokens：金额与 tokens（仅非管理员令牌且有 usage/金额时）
    if let Some(tok) = client_token.filter(|t| *t != "admin_token") {
        if let Some(delta) = amount_spent
            && let Err(e) = app_state.token_store.add_amount_spent(tok, delta).await
        {
            tracing::warn!("Failed to update token spent: {}", e);
        }
        if let Ok(r) = response && let Some(u) = r.typed.usage.as_ref() {
            let prompt = u.prompt_tokens as i64;
            let completion = u.completion_tokens as i64;
            let total = u.total_tokens as i64;
            if let Err(e) = app_state
                .token_store
                .add_usage_spent(tok, prompt, completion, total)
                .await
            {
                tracing::warn!("Failed to update token tokens: {}", e);
            }
        }
    }
}

// 记录普通请求（不含 tokens）
#[allow(clippy::too_many_arguments)]
pub async fn log_simple_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    method: &str,
    path: &str,
    request_type: &str,
    model: Option<String>,
    provider: Option<String>,
    client_token: Option<&str>,
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
        client_token: client_token.map(|s| s.to_string()),
        amount_spent: None,
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
