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
use crate::logging::types::RequestLogDetailRecord;
use crate::server::AppState;
use crate::server::model_display::format_model_display_name;
use crate::server::request_logging::log_simple_request;

const MAX_LOG_LIMIT: usize = 1000;
const DEFAULT_LOG_LIMIT: usize = 200;
const CLIENT_TOKEN_ID_PREFIX: &str = "atk_";
const RECHARGE_AMOUNT_CURRENCY: &str = "CNY";

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
    pub requested_model_display: Option<String>,
    pub effective_model_display: Option<String>,
    pub model_display: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub client_token_id: Option<String>,
    pub client_token_name: Option<String>,
    pub username: Option<String>,
    pub amount_spent: Option<f64>,
    pub amount_spent_currency: Option<String>,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub error_message: Option<String>,
    pub success: bool,
    pub replayable: bool,
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

#[derive(Debug, Clone)]
enum NormalizedClientToken {
    TokenId(String),
    AdminIdentity(&'static str),
}

fn normalize_client_token(raw: Option<&str>) -> Option<NormalizedClientToken> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with(CLIENT_TOKEN_ID_PREFIX) {
        return Some(NormalizedClientToken::TokenId(raw.to_string()));
    }
    if raw == "jwt" {
        return Some(NormalizedClientToken::AdminIdentity("jwt"));
    }
    if raw == "web_session" {
        return Some(NormalizedClientToken::AdminIdentity("web_session"));
    }
    if raw == "tui_session" {
        return Some(NormalizedClientToken::AdminIdentity("tui_session"));
    }
    // 历史脏数据：直接写入 JWT 明文串（含 '.' 且通常以 'eyJ' 开头）
    if raw.starts_with("eyJ") && raw.contains('.') {
        return Some(NormalizedClientToken::AdminIdentity("jwt"));
    }
    // 历史脏数据：写入了明文 Client Token；响应层归一化为不可逆 token_id
    Some(NormalizedClientToken::TokenId(
        crate::admin::client_token_id_for_token(raw),
    ))
}

fn normalize_logged_price_currency(currency: Option<&str>) -> String {
    match currency
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("USD")
        .to_ascii_uppercase()
        .as_str()
    {
        "RMB" | "CNH" => "CNY".to_string(),
        other => other.to_string(),
    }
}

fn resolve_amount_spent_currency(
    request_type: &str,
    provider: Option<&str>,
    billing_model: Option<&str>,
    effective_model: Option<&str>,
    requested_model: Option<&str>,
    price_currency_by_key: &std::collections::HashMap<String, Option<String>>,
) -> Option<String> {
    if request_type == "recharge"
        || request_type.starts_with("recharge_")
        || request_type == "subscription_purchase"
    {
        return Some(RECHARGE_AMOUNT_CURRENCY.to_string());
    }

    let provider = provider.map(str::trim).filter(|value| !value.is_empty())?;

    for model in [billing_model, effective_model, requested_model] {
        let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        let key = format!("{provider}:{model}");
        if let Some(currency) = price_currency_by_key.get(&key) {
            return Some(normalize_logged_price_currency(currency.as_deref()));
        }
    }

    None
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
                matches(&log.requested_model)
                    || matches(&log.effective_model)
                    || matches(&log.model)
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

fn is_replayable_detail(detail: Option<&RequestLogDetailRecord>) -> bool {
    detail
        .and_then(|item| item.request_payload_snapshot.as_deref())
        .map(|snapshot| !snapshot.trim().is_empty())
        .unwrap_or(false)
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
    let token_meta_by_id = {
        use std::collections::HashMap;
        #[derive(Clone)]
        struct TokenMeta {
            name: String,
            user_id: Option<String>,
        }
        let mut map: HashMap<String, TokenMeta> = HashMap::new();
        if let Ok(tokens) = app_state.token_store.list_tokens().await {
            for t in tokens {
                map.insert(
                    t.id,
                    TokenMeta {
                        name: t.name,
                        user_id: t.user_id,
                    },
                );
            }
        }
        map
    };
    let username_by_user_id = {
        use std::collections::{HashMap, HashSet};
        let mut map: HashMap<String, String> = HashMap::new();
        if let Ok(users) = app_state.user_store.list_users().await {
            for u in users {
                map.insert(u.id, u.username);
            }
            map
        } else {
            // Fallback: only resolve usernames that actually appear in current log batch.
            let mut needed: HashSet<String> = HashSet::new();
            for l in filtered.iter() {
                if let Some(uid) = l.user_id.as_ref() {
                    needed.insert(uid.clone());
                }
            }
            for uid in needed.into_iter() {
                if let Ok(Some(u)) = app_state.user_store.get_user(&uid).await {
                    map.insert(u.id, u.username);
                }
            }
            map
        }
    };
    let providers_by_id: std::collections::HashMap<String, crate::config::settings::Provider> =
        app_state
            .providers
            .list_providers()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|provider| (provider.name.clone(), provider))
            .collect();
    let price_currency_by_key = app_state
        .log_store
        .list_model_prices(None)
        .await
        .map(|items| {
            items.into_iter().fold(
                std::collections::HashMap::<String, Option<String>>::new(),
                |mut acc, item| {
                    acc.insert(format!("{}:{}", item.provider, item.model), item.currency);
                    acc
                },
            )
        })
        .unwrap_or_default();
    let mut replayable_by_id: std::collections::HashMap<i64, bool> =
        std::collections::HashMap::new();
    for log in &filtered {
        let replayable = if log.request_type.starts_with("chat_") {
            if let Some(log_id) = log.id {
                let detail = app_state
                    .log_store
                    .get_request_log_detail(log_id)
                    .await
                    .map_err(GatewayError::Db)?;
                is_replayable_detail(detail.as_ref())
            } else {
                false
            }
        } else {
            false
        };
        if let Some(log_id) = log.id {
            replayable_by_id.insert(log_id, replayable);
        }
    }
    let data: Vec<RequestLogEntry> = filtered
        .into_iter()
        .map(|log| {
            let username_from_user_id = log
                .user_id
                .as_ref()
                .and_then(|uid| username_by_user_id.get(uid).cloned());
            // 注意：严禁返回 token/JWT 明文；这里始终对 client_token 做归一化输出
            // - Token / 明文 token -> client_token_id = atk_...；client_token_name = token.name(或 fallback)
            // - 管理员身份 / JWT -> client_token_id = null；client_token_name = 管理员(...)
            let normalized = normalize_client_token(log.client_token.as_deref());
            let (client_token_id, client_token_name, username) = match normalized {
                Some(NormalizedClientToken::TokenId(id)) => {
                    let meta = token_meta_by_id.get(&id);
                    let name = meta
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| crate::admin::normalize_client_token_name(None, &id));
                    let username = meta
                        .and_then(|m| m.user_id.as_ref())
                        .and_then(|user_id| username_by_user_id.get(user_id).cloned());
                    (Some(id), Some(name), username)
                }
                Some(NormalizedClientToken::AdminIdentity(kind)) => {
                    (None, Some(format!("管理员({})", kind)), None)
                }
                None => (None, None, username_from_user_id),
            };
            let requested_model_raw = log.requested_model.clone().or_else(|| log.model.clone());
            let effective_model_raw = log.effective_model.clone().or_else(|| log.model.clone());
            let amount_spent_currency = resolve_amount_spent_currency(
                &log.request_type,
                log.provider.as_deref(),
                log.model.as_deref(),
                effective_model_raw.as_deref(),
                requested_model_raw.as_deref(),
                &price_currency_by_key,
            );
            let requested_model_display = requested_model_raw.as_deref().map(|model| {
                format_model_display_name(&providers_by_id, model, log.provider.as_deref())
            });
            let effective_model_display = effective_model_raw.as_deref().map(|model| {
                format_model_display_name(&providers_by_id, model, log.provider.as_deref())
            });
            let model_display = effective_model_display
                .clone()
                .or_else(|| requested_model_display.clone());
            let requested_model = requested_model_display.clone().or(requested_model_raw);
            let effective_model = effective_model_display.clone().or(effective_model_raw);

            RequestLogEntry {
                id: log.id,
                timestamp: log.timestamp.to_rfc3339(),
                method: log.method.clone(),
                path: log.path.clone(),
                request_type: log.request_type.clone(),
                requested_model,
                effective_model,
                requested_model_display,
                effective_model_display,
                model_display,
                provider: log.provider.clone(),
                api_key: log.api_key.clone(),
                client_token_id,
                client_token_name,
                username,
                amount_spent: log.amount_spent,
                amount_spent_currency,
                status_code: log.status_code,
                response_time_ms: log.response_time_ms,
                prompt_tokens: log.prompt_tokens,
                completion_tokens: log.completion_tokens,
                total_tokens: log.total_tokens,
                cached_tokens: log.cached_tokens,
                reasoning_tokens: log.reasoning_tokens,
                error_message: log.error_message.clone(),
                success: log.status_code < 400,
                replayable: log
                    .id
                    .and_then(|log_id| replayable_by_id.get(&log_id).copied())
                    .unwrap_or(false),
            }
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

    let token_meta_by_id = {
        use std::collections::HashMap;
        #[derive(Clone)]
        struct TokenMeta {
            name: String,
            user_id: Option<String>,
        }
        let mut map: HashMap<String, TokenMeta> = HashMap::new();
        if let Ok(tokens) = app_state.token_store.list_tokens().await {
            for t in tokens {
                map.insert(
                    t.id,
                    TokenMeta {
                        name: t.name,
                        user_id: t.user_id,
                    },
                );
            }
        }
        map
    };
    let username_by_user_id = {
        use std::collections::HashMap;
        let mut map: HashMap<String, String> = HashMap::new();
        if let Ok(users) = app_state.user_store.list_users().await {
            for u in users {
                map.insert(u.id, u.username);
            }
        }
        map
    };
    let providers_by_id: std::collections::HashMap<String, crate::config::settings::Provider> =
        app_state
            .providers
            .list_providers()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|provider| (provider.name.clone(), provider))
            .collect();
    let price_currency_by_key = app_state
        .log_store
        .list_model_prices(None)
        .await
        .map(|items| {
            items.into_iter().fold(
                std::collections::HashMap::<String, Option<String>>::new(),
                |mut acc, item| {
                    acc.insert(format!("{}:{}", item.provider, item.model), item.currency);
                    acc
                },
            )
        })
        .unwrap_or_default();
    let data: Vec<RequestLogEntry> = raw_logs
        .iter()
        .map(|log| {
            let normalized = normalize_client_token(log.client_token.as_deref());
            let (client_token_id, client_token_name, username) = match normalized {
                Some(NormalizedClientToken::TokenId(id)) => {
                    let meta = token_meta_by_id.get(&id);
                    let name = meta
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| crate::admin::normalize_client_token_name(None, &id));
                    let username = meta
                        .and_then(|m| m.user_id.as_ref())
                        .and_then(|user_id| username_by_user_id.get(user_id).cloned());
                    (Some(id), Some(name), username)
                }
                Some(NormalizedClientToken::AdminIdentity(kind)) => {
                    (None, Some(format!("管理员({})", kind)), None)
                }
                None => (None, None, None),
            };
            let requested_model_raw = log.requested_model.clone().or_else(|| log.model.clone());
            let effective_model_raw = log.effective_model.clone().or_else(|| log.model.clone());
            let amount_spent_currency = resolve_amount_spent_currency(
                &log.request_type,
                log.provider.as_deref(),
                log.model.as_deref(),
                effective_model_raw.as_deref(),
                requested_model_raw.as_deref(),
                &price_currency_by_key,
            );
            let requested_model_display = requested_model_raw.as_deref().map(|model| {
                format_model_display_name(&providers_by_id, model, log.provider.as_deref())
            });
            let effective_model_display = effective_model_raw.as_deref().map(|model| {
                format_model_display_name(&providers_by_id, model, log.provider.as_deref())
            });
            let model_display = effective_model_display
                .clone()
                .or_else(|| requested_model_display.clone());
            let requested_model = requested_model_display.clone().or(requested_model_raw);
            let effective_model = effective_model_display.clone().or(effective_model_raw);

            RequestLogEntry {
                id: log.id,
                timestamp: log.timestamp.to_rfc3339(),
                method: log.method.clone(),
                path: log.path.clone(),
                request_type: log.request_type.clone(),
                requested_model,
                effective_model,
                requested_model_display,
                effective_model_display,
                model_display,
                provider: log.provider.clone(),
                api_key: log.api_key.clone(),
                client_token_id,
                client_token_name,
                username,
                amount_spent: log.amount_spent,
                amount_spent_currency,
                status_code: log.status_code,
                response_time_ms: log.response_time_ms,
                prompt_tokens: log.prompt_tokens,
                completion_tokens: log.completion_tokens,
                total_tokens: log.total_tokens,
                cached_tokens: log.cached_tokens,
                reasoning_tokens: log.reasoning_tokens,
                error_message: log.error_message.clone(),
                success: log.status_code < 400,
                replayable: false,
            }
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
