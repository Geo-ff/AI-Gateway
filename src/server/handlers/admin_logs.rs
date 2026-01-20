use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::auth::{AdminIdentity, require_superadmin};
use crate::error::GatewayError;
use crate::logging::types::RequestLog;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;

const MAX_LOG_LIMIT: usize = 1000;
const DEFAULT_LOG_LIMIT: usize = 200;

#[derive(Debug, Deserialize, Default)]
pub struct OpsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LogsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(default)]
    pub request_type: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub client_token: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub status: Option<String>, // success | error
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RequestLogEntry {
    pub id: Option<i64>,
    pub timestamp: String,
    pub method: String,
    pub path: String,
    pub request_type: String,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub client_token_id: Option<String>,
    pub client_token_name: Option<String>,
    pub amount_spent: Option<f64>,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub error_message: Option<String>,
    pub success: bool,
}

#[derive(Debug, Serialize)]
pub struct RequestLogsResponse {
    pub total: usize,
    pub data: Vec<RequestLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ChatCompletionsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cursor: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OperationLogEntry {
    pub id: Option<i64>,
    pub timestamp: String,
    pub operation: String,
    pub provider: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OperationLogsResponse {
    pub total: usize,
    pub data: Vec<OperationLogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
}

fn identity_label(identity: &AdminIdentity) -> &'static str {
    match identity {
        AdminIdentity::Jwt(_) => "jwt",
        AdminIdentity::TuiSession(_) => "tui_session",
        AdminIdentity::WebSession(_) => "web_session",
    }
}

fn filter_logs<'a>(logs: &'a [RequestLog], query: &LogsQuery) -> Vec<&'a RequestLog> {
    logs.iter()
        .filter(|log| match query.request_type.as_ref() {
            Some(rt) => log.request_type.eq_ignore_ascii_case(rt),
            None => true,
        })
        .filter(|log| match query.method.as_ref() {
            Some(m) => log.method.eq_ignore_ascii_case(m),
            None => true,
        })
        .filter(|log| match query.path.as_ref() {
            Some(p) => log.path.eq_ignore_ascii_case(p),
            None => true,
        })
        .filter(|log| match query.provider.as_ref() {
            Some(provider) => log
                .provider
                .as_ref()
                .map(|p| p.eq_ignore_ascii_case(provider))
                .unwrap_or(false),
            None => true,
        })
        .filter(|log| match query.model.as_ref() {
            Some(model) => {
                let matches = |v: &Option<String>| {
                    v.as_ref()
                        .map(|m| m.eq_ignore_ascii_case(model))
                        .unwrap_or(false)
                };
                matches(&log.requested_model) || matches(&log.effective_model) || matches(&log.model)
            }
            None => true,
        })
        .filter(|log| match query.client_token.as_ref() {
            Some(token) => log
                .client_token
                .as_ref()
                .map(|t| t == token)
                .unwrap_or(false),
            None => true,
        })
        .filter(|log| match query.api_key.as_ref() {
            Some(api_key) => log.api_key.as_ref().map(|k| k == api_key).unwrap_or(false),
            None => true,
        })
        .filter(|log| match query.status.as_deref() {
            Some("success") => log.status_code < 400,
            Some("error") => log.status_code >= 400,
            Some(_) => true,
            None => true,
        })
        .collect()
}

pub async fn list_request_logs(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<LogsQuery>,
) -> Result<Json<RequestLogsResponse>, GatewayError> {
    let identity = require_superadmin(&headers, &app_state).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);

    // 若存在任何筛选条件，则分批读取并筛选，直到凑满 limit 或无更多数据
    let has_filters = query.request_type.is_some()
        || query.provider.is_some()
        || query.model.is_some()
        || query.client_token.is_some()
        || query.api_key.is_some()
        || query.status.is_some()
        || query.method.is_some()
        || query.path.is_some();

    let (raw_logs, next_cursor) = if has_filters {
        let (logs, next_cur) =
            get_logs_matching_query(&app_state, limit as i32, query.cursor, &query).await?;
        (logs, next_cur)
    } else {
        let logs = app_state
            .log_store
            .get_recent_logs_with_cursor(limit as i32, query.cursor)
            .await
            .map_err(GatewayError::Db)?;
        let next_cur = logs
            .last()
            .and_then(|l| l.id)
            .filter(|_| logs.len() as usize == limit);
        (logs, next_cur)
    };

    let filtered = filter_logs(&raw_logs, &query);
    let name_by_id = {
        use std::collections::HashMap;
        let mut map: HashMap<String, String> = HashMap::new();
        if let Ok(tokens) = app_state.token_store.list_tokens().await {
            for t in tokens {
                map.insert(t.id, t.name);
            }
        }
        map
    };
    let data: Vec<RequestLogEntry> = filtered
        .into_iter()
        .map(|log| RequestLogEntry {
            id: log.id,
            timestamp: log.timestamp.to_rfc3339(),
            method: log.method.clone(),
            path: log.path.clone(),
            request_type: log.request_type.clone(),
            requested_model: log
                .requested_model
                .clone()
                .or_else(|| log.model.clone()),
            effective_model: log
                .effective_model
                .clone()
                .or_else(|| log.model.clone()),
            provider: log.provider.clone(),
            api_key: log.api_key.clone(),
            client_token_id: log.client_token.clone(),
            client_token_name: log.client_token.as_deref().map(|id| {
                name_by_id
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| crate::admin::normalize_client_token_name(None, id))
            }),
            amount_spent: log.amount_spent,
            status_code: log.status_code,
            response_time_ms: log.response_time_ms,
            prompt_tokens: log.prompt_tokens,
            completion_tokens: log.completion_tokens,
            total_tokens: log.total_tokens,
            cached_tokens: log.cached_tokens,
            reasoning_tokens: log.reasoning_tokens,
            error_message: log.error_message.clone(),
            success: log.status_code < 400,
        })
        .collect();

    // next_cursor 已在上方计算，避免重复绑定

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/logs/requests",
        "admin_logs_requests",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(RequestLogsResponse {
        total: data.len(),
        data,
        next_cursor,
    }))
}

pub async fn list_chat_completion_logs(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ChatCompletionsQuery>,
) -> Result<Json<RequestLogsResponse>, GatewayError> {
    let identity = require_superadmin(&headers, &app_state).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT) as i32;

    let raw_logs = app_state
        .log_store
        .get_logs_by_method_path("POST", "/v1/chat/completions", limit, query.cursor)
        .await
        .map_err(GatewayError::Db)?;

    let name_by_id = {
        use std::collections::HashMap;
        let mut map: HashMap<String, String> = HashMap::new();
        if let Ok(tokens) = app_state.token_store.list_tokens().await {
            for t in tokens {
                map.insert(t.id, t.name);
            }
        }
        map
    };
    let data: Vec<RequestLogEntry> = raw_logs
        .iter()
        .map(|log| RequestLogEntry {
            id: log.id,
            timestamp: log.timestamp.to_rfc3339(),
            method: log.method.clone(),
            path: log.path.clone(),
            request_type: log.request_type.clone(),
            requested_model: log
                .requested_model
                .clone()
                .or_else(|| log.model.clone()),
            effective_model: log
                .effective_model
                .clone()
                .or_else(|| log.model.clone()),
            provider: log.provider.clone(),
            api_key: log.api_key.clone(),
            client_token_id: log.client_token.clone(),
            client_token_name: log.client_token.as_deref().map(|id| {
                name_by_id
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| crate::admin::normalize_client_token_name(None, id))
            }),
            amount_spent: log.amount_spent,
            status_code: log.status_code,
            response_time_ms: log.response_time_ms,
            prompt_tokens: log.prompt_tokens,
            completion_tokens: log.completion_tokens,
            total_tokens: log.total_tokens,
            cached_tokens: log.cached_tokens,
            reasoning_tokens: log.reasoning_tokens,
            error_message: log.error_message.clone(),
            success: log.status_code < 400,
        })
        .collect();

    let next_cursor = raw_logs
        .last()
        .and_then(|l| l.id)
        .filter(|_| raw_logs.len() as i32 == limit);

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/logs/chat-completions",
        "admin_logs_chat_completions",
        None,
        None,
        Some(match identity {
            AdminIdentity::Jwt(_) => "jwt",
            AdminIdentity::TuiSession(_) => "tui_session",
            AdminIdentity::WebSession(_) => "web_session",
        }),
        200,
        None,
    )
    .await;

    Ok(Json(RequestLogsResponse {
        total: data.len(),
        data,
        next_cursor,
    }))
}

// 带任意筛选条件的分页筛选：不断向后分页，直到填满 limit 或耗尽为止
async fn get_logs_matching_query(
    app_state: &Arc<AppState>,
    limit: i32,
    cursor: Option<i64>,
    query: &LogsQuery,
) -> Result<(Vec<RequestLog>, Option<i64>), GatewayError> {
    let page_size = limit.max(100); // 至少读取 100 条以提高命中率
    let mut acc: Vec<RequestLog> = Vec::with_capacity(limit as usize);
    let mut next = cursor;

    loop {
        let batch = app_state
            .log_store
            .get_recent_logs_with_cursor(page_size, next)
            .await
            .map_err(GatewayError::Db)?;
        if batch.is_empty() {
            break;
        }
        // 筛选本批次
        for l in batch.iter() {
            // 复用现有过滤逻辑
            if filter_logs(std::slice::from_ref(l), query).is_empty() {
                continue;
            }
            acc.push(l.clone());
            if acc.len() >= limit as usize {
                // 使用最后一条实际返回的日志 ID 作为下一游标，避免回跳
                let last_cursor = acc.last().and_then(|x| x.id);
                return Ok((acc, last_cursor));
            }
        }
        // 继续下一批
        next = batch.last().and_then(|b| b.id);
        if batch.len() < page_size as usize {
            break;
        }
    }

    Ok((acc, None))
}

pub async fn list_operation_logs(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<OpsQuery>,
) -> Result<Json<OperationLogsResponse>, GatewayError> {
    let identity = require_superadmin(&headers, &app_state).await?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);
    let raw_logs = app_state
        .log_store
        .get_provider_ops_logs(limit as i32, query.cursor)
        .await
        .map_err(GatewayError::Db)?;

    let filtered = raw_logs
        .iter()
        .filter(|log| match query.operation.as_ref() {
            Some(op) => log.operation.eq_ignore_ascii_case(op),
            None => true,
        })
        .filter(|log| match query.provider.as_ref() {
            Some(p) => log
                .provider
                .as_ref()
                .map(|v| v.eq_ignore_ascii_case(p))
                .unwrap_or(false),
            None => true,
        })
        .collect::<Vec<_>>();

    let data = filtered
        .iter()
        .map(|log| OperationLogEntry {
            id: log.id,
            timestamp: log.timestamp.to_rfc3339(),
            operation: log.operation.clone(),
            provider: log.provider.clone(),
            details: log.details.clone(),
        })
        .collect::<Vec<_>>();

    let next_cursor = raw_logs
        .last()
        .and_then(|log| log.id)
        .filter(|_| raw_logs.len() as usize == limit);

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/logs/operations",
        "admin_logs_operations",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(OperationLogsResponse {
        total: data.len(),
        data,
        next_cursor,
    }))
}
