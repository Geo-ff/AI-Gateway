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
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};

pub async fn get_balance(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
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
                "/me/balance",
                "me_balance_get",
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

    let user = app_state
        .user_store
        .get_user(&claims.sub)
        .await?
        .ok_or_else(|| GatewayError::Unauthorized("invalid credentials".into()))?;

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/balance",
        "me_balance_get",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "balance": user.balance,
    })))
}

#[derive(Debug, Deserialize)]
pub struct TransactionsQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

pub async fn list_transactions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<TransactionsQuery>,
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
                "/me/balance/transactions",
                "me_balance_transactions",
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
    let offset = q.offset.unwrap_or(0).max(0);

    let user = app_state
        .user_store
        .get_user(&claims.sub)
        .await?
        .ok_or_else(|| GatewayError::Unauthorized("invalid credentials".into()))?;

    let items = app_state
        .balance_store
        .list_transactions(&claims.sub, limit, offset)
        .await?
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "kind": t.kind.as_str(),
                "amount": t.amount,
                "created_at": crate::logging::time::to_iso8601_utc_string(&t.created_at),
                "meta": t.meta,
            })
        })
        .collect::<Vec<_>>();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/balance/transactions",
        "me_balance_transactions",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;

    Ok(Json(serde_json::json!({
        "balance": user.balance,
        "limit": limit,
        "offset": offset,
        "items": items,
    })))
}
