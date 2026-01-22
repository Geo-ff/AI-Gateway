use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use super::auth::require_user;
use crate::error::GatewayError;
use crate::logging::types::RequestLog;
use crate::server::AppState;

const MAX_LOG_LIMIT: usize = 1000;
const DEFAULT_LOG_LIMIT: usize = 200;

#[derive(Debug, Deserialize, Default)]
pub struct MyLogsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(rename = "type", default)]
    pub log_type: Option<String>, // consumption | recharge
    #[serde(default)]
    pub status: Option<String>, // success | failed | error
    #[serde(default)]
    pub filter: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MyRequestLogEntry {
    pub id: Option<i64>,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub request_type: String,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub client_token_id: Option<String>,
    pub client_token_name: Option<String>,
    pub amount_spent: Option<f64>,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub error_message: Option<String>,
    pub success: bool,
}

#[derive(Debug, Serialize)]
pub struct MyRequestLogsResponse {
    pub total: usize,
    pub data: Vec<MyRequestLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

fn derive_log_type(request_type: &str) -> Option<&'static str> {
    if request_type.starts_with("chat_") {
        return Some("consumption");
    }
    if request_type == "recharge" || request_type.starts_with("recharge_") {
        return Some("recharge");
    }
    if request_type == "subscription_purchase" {
        return Some("recharge");
    }
    None
}

fn normalize_status(status_code: u16) -> &'static str {
    if status_code < 400 {
        "success"
    } else {
        "failed"
    }
}

fn matches_query(
    log: &RequestLog,
    log_type: &str,
    token_id: Option<&str>,
    token_name: Option<&str>,
    q: &MyLogsQuery,
) -> bool {
    if let Some(want_type) = q
        .log_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if want_type != log_type {
            return false;
        }
    }

    if let Some(want_status) = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let normalized = normalize_status(log.status_code);
        let want = if want_status == "error" {
            "failed"
        } else {
            want_status
        };
        if want != normalized {
            return false;
        }
    }

    let Some(filter) = q.filter.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let filter = filter.to_lowercase();

    let token_id = token_id.unwrap_or("").to_lowercase();
    let token_name = token_name.unwrap_or("").to_lowercase();
    let model = log
        .effective_model
        .as_ref()
        .or(log.requested_model.as_ref())
        .or(log.model.as_ref())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    token_id.contains(&filter) || token_name.contains(&filter) || model.contains(&filter)
}

pub async fn list_my_request_logs(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<MyLogsQuery>,
) -> Result<Json<MyRequestLogsResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let user_id = claims.sub;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);

    let tokens = app_state.token_store.list_tokens_by_user(&user_id).await?;
    let token_ids: HashSet<String> = tokens.iter().map(|t| t.id.clone()).collect();
    let token_name_by_id: HashMap<String, String> =
        tokens.into_iter().map(|t| (t.id, t.name)).collect();

    let mut out: Vec<RequestLog> = Vec::with_capacity(limit);
    let mut next = query.cursor;
    let page_size = (limit.max(100)) as i32;

    loop {
        let batch = app_state
            .log_store
            .get_recent_logs_with_cursor(page_size, next)
            .await
            .map_err(GatewayError::Db)?;
        if batch.is_empty() {
            break;
        }

        for l in batch.iter() {
            let belongs_to_user = l.user_id.as_deref().is_some_and(|uid| uid == user_id);
            let token_id = l.client_token.as_deref();
            let belongs_to_token = token_id.is_some_and(|id| token_ids.contains(id));
            if !belongs_to_user && !belongs_to_token {
                continue;
            }

            let Some(log_type) = derive_log_type(&l.request_type) else {
                continue;
            };

            let token_name = token_id.and_then(|id| token_name_by_id.get(id).map(String::as_str));

            if !matches_query(l, log_type, token_id, token_name, &query) {
                continue;
            }

            out.push(l.clone());
            if out.len() >= limit {
                break;
            }
        }

        if out.len() >= limit {
            break;
        }

        next = batch.last().and_then(|b| b.id);
        if batch.len() < page_size as usize {
            break;
        }
    }

    let next_cursor = out.last().and_then(|l| l.id).filter(|_| out.len() == limit);

    let data = out
        .into_iter()
        .map(|log| {
            let token_id = log
                .client_token
                .as_deref()
                .filter(|id| token_ids.contains(*id));
            let token_name = token_id.and_then(|id| token_name_by_id.get(id).cloned());
            MyRequestLogEntry {
                id: log.id,
                timestamp: log.timestamp.to_rfc3339(),
                method: log.method,
                path: log.path,
                request_type: log.request_type,
                requested_model: log.requested_model.clone().or_else(|| log.model.clone()),
                effective_model: log.effective_model.clone().or_else(|| log.model.clone()),
                client_token_id: token_id.map(|s| s.to_string()),
                client_token_name: token_name,
                amount_spent: log.amount_spent,
                status_code: log.status_code,
                response_time_ms: log.response_time_ms,
                prompt_tokens: log.prompt_tokens,
                completion_tokens: log.completion_tokens,
                total_tokens: log.total_tokens,
                error_message: log.error_message,
                success: log.status_code < 400,
            }
        })
        .collect::<Vec<_>>();

    Ok(Json(MyRequestLogsResponse {
        total: data.len(),
        data,
        next_cursor,
    }))
}
