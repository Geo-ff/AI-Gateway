use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use super::auth::require_user;
use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_CHAT_ONCE, REQ_TYPE_CHAT_STREAM};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Deserialize)]
pub struct BalanceQuery {
    #[serde(default)]
    pub token_id: Option<String>,
}

pub async fn my_token_balance(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<BalanceQuery>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/token/balance",
                "me_token_balance",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let tokens = app_state.token_store.list_tokens_by_user(&claims.sub).await?;
    if let Some(token_id) = q.token_id.as_deref() {
        let Some(t) = tokens.into_iter().find(|t| t.id == token_id) else {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/token/balance",
                "me_token_balance",
                None,
                None,
                provided.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        };
        let spent = t.amount_spent;
        let remaining = t.max_amount.map(|m| (m - spent).max(0.0));
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/me/token/balance",
            "me_token_balance",
            None,
            None,
            token_for_log(provided.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(Json(serde_json::json!({
            "token_id": t.id,
            "amount_spent": spent,
            "max_amount": t.max_amount,
            "remaining": remaining,
            "total_tokens_spent": t.total_tokens_spent,
            "max_tokens": t.max_tokens,
        })));
    }

    let mut total_amount_spent = 0.0;
    let mut total_remaining = 0.0;
    let mut has_unlimited = false;
    let items: Vec<_> = tokens
        .into_iter()
        .map(|t| {
            total_amount_spent += t.amount_spent;
            let remaining = match t.max_amount {
                Some(max) => {
                    let r = (max - t.amount_spent).max(0.0);
                    total_remaining += r;
                    Some(r)
                }
                None => {
                    has_unlimited = true;
                    None
                }
            };
            serde_json::json!({
                "token_id": t.id,
                "name": t.name,
                "amount_spent": t.amount_spent,
                "max_amount": t.max_amount,
                "remaining": remaining,
                "total_tokens_spent": t.total_tokens_spent,
                "max_tokens": t.max_tokens,
                "enabled": t.enabled,
                "expires_at": t.expires_at.as_ref().map(crate::logging::time::to_beijing_string),
                "created_at": crate::logging::time::to_beijing_string(&t.created_at),
            })
        })
        .collect();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/token/balance",
        "me_token_balance",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "total_amount_spent": total_amount_spent,
        "total_remaining": total_remaining,
        "has_unlimited": has_unlimited,
        "items": items,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    #[serde(default)]
    pub token_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i32>,
}

pub async fn my_token_usage(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<UsageQuery>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/token/usage",
                "me_token_usage",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let fetch_limit = (limit * 5).min(1000);
    let tokens = app_state.token_store.list_tokens_by_user(&claims.sub).await?;
    let selected: Vec<_> = if let Some(token_id) = q.token_id.as_deref() {
        let Some(t) = tokens.iter().find(|t| t.id == token_id) else {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/token/usage",
                "me_token_usage",
                None,
                None,
                provided.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        };
        vec![t.clone()]
    } else {
        tokens
    };

    let mut all_logs = Vec::new();
    for t in selected.iter() {
        let logs = app_state
            .log_store
            .get_logs_by_client_token(&t.token, fetch_limit)
            .await
            .map_err(GatewayError::Db)?;
        for l in logs.into_iter() {
            if l.request_type != REQ_TYPE_CHAT_ONCE && l.request_type != REQ_TYPE_CHAT_STREAM {
                continue;
            }
            all_logs.push((t.id.clone(), l));
        }
    }
    all_logs.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));

    let mut chat_items = Vec::with_capacity(limit as usize);
    for (token_id, l) in all_logs.into_iter() {
        chat_items.push(serde_json::json!({
            "token_id": token_id,
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

    let total_cost: f64 = selected.iter().map(|t| t.amount_spent).sum();
    let prompt_tokens_spent: i64 = selected.iter().map(|t| t.prompt_tokens_spent).sum();
    let completion_tokens_spent: i64 = selected.iter().map(|t| t.completion_tokens_spent).sum();
    let total_tokens_spent: i64 = selected.iter().map(|t| t.total_tokens_spent).sum();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/token/usage",
        "me_token_usage",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "token_ids": selected.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
        "limit": limit,
        "total_cost": total_cost,
        "prompt_tokens_spent": prompt_tokens_spent,
        "completion_tokens_spent": completion_tokens_spent,
        "total_tokens_spent": total_tokens_spent,
        "items": chat_items,
    })))
}

