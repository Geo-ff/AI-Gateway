use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_CHAT_ONCE, REQ_TYPE_CHAT_STREAM};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use chrono::Utc;

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

async fn ensure_active_token(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<String, GatewayError> {
    let Some(tok) = bearer(headers) else {
        return Err(GatewayError::Config("missing bearer token".into()));
    };
    if let Some(t) = app_state.token_store.get_token(&tok).await? {
        if !t.enabled {
            return Err(GatewayError::Config("token disabled".into()));
        }
        if let Some(exp) = t.expires_at
            && chrono::Utc::now() > exp
        {
            return Err(GatewayError::Config("token expired".into()));
        }
        Ok(tok)
    } else {
        Err(GatewayError::Config("invalid token".into()))
    }
}

pub async fn token_balance(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer(&headers);
    let provided_for_log = provided.as_deref();
    let token = match ensure_active_token(&headers, &app_state).await {
        Ok(t) => t,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/v1/token/balance",
                "token_balance",
                None,
                None,
                provided_for_log,
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };
    // 使用 client_tokens.amount_spent 作为权威累计消费
    let token_row = app_state.token_store.get_token(&token).await?;
    let spent = token_row.as_ref().map(|t| t.amount_spent).unwrap_or(0.0);
    let max_amount = token_row.as_ref().and_then(|t| t.max_amount);
    let total_tokens_spent = token_row
        .as_ref()
        .map(|t| t.total_tokens_spent)
        .unwrap_or(0);
    let max_tokens = token_row.as_ref().and_then(|t| t.max_tokens);
    let remaining = max_amount.map(|m| (m - spent).max(0.0));
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/v1/token/balance",
        "token_balance",
        None,
        None,
        Some(token.as_str()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "token": token,
        "amount_spent": spent,
        "max_amount": max_amount,
        "remaining": remaining,
        "total_tokens_spent": total_tokens_spent,
        "max_tokens": max_tokens,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    #[serde(default)]
    pub limit: Option<i32>,
}

pub async fn token_usage(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<UsageQuery>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer(&headers);
    let provided_for_log = provided.as_deref();
    let token = match ensure_active_token(&headers, &app_state).await {
        Ok(t) => t,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/v1/token/usage",
                "token_usage",
                None,
                None,
                provided_for_log,
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    // 为避免暴露非聊天日志，这里拉取更大的窗口后过滤为聊天类型
    let fetch_limit = (limit * 5).min(1000);
    let logs = app_state
        .log_store
        .get_logs_by_client_token(&token, fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let mut chat_items = Vec::with_capacity(limit as usize);
    for l in logs.into_iter() {
        if l.request_type != REQ_TYPE_CHAT_ONCE && l.request_type != REQ_TYPE_CHAT_STREAM {
            continue;
        }
        chat_items.push(serde_json::json!({
            "timestamp": crate::logging::time::to_beijing_string(&l.timestamp),
            "provider": l.provider,
            "model": l.model,
            "status_code": l.status_code,
            "response_time_ms": l.response_time_ms,
            "prompt_tokens": l.prompt_tokens,
            "completion_tokens": l.completion_tokens,
            "total_tokens": l.total_tokens,
            "amount_spent": l.amount_spent,
            "error_message": l.error_message,
        }));
        if chat_items.len() as i32 >= limit {
            break;
        }
    }
    // 总消费额度取自 client_tokens.amount_spent（权威聚合值）
    let token_row = app_state.token_store.get_token(&token).await?;
    let total_cost = token_row.as_ref().map(|t| t.amount_spent).unwrap_or(0.0);
    let prompt_tokens_spent = token_row
        .as_ref()
        .map(|t| t.prompt_tokens_spent)
        .unwrap_or(0);
    let completion_tokens_spent = token_row
        .as_ref()
        .map(|t| t.completion_tokens_spent)
        .unwrap_or(0);
    let total_tokens_spent = token_row
        .as_ref()
        .map(|t| t.total_tokens_spent)
        .unwrap_or(0);
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/v1/token/usage",
        "token_usage",
        None,
        None,
        Some(token.as_str()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "token": token,
        "limit": limit,
        "total_cost": total_cost,
        "prompt_tokens_spent": prompt_tokens_spent,
        "completion_tokens_spent": completion_tokens_spent,
        "total_tokens_spent": total_tokens_spent,
        "items": chat_items,
    })))
}
