use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::error::GatewayError;
use crate::logging::types::{
    ProviderOpLog, REQ_TYPE_PROVIDER_KEY_ADD, REQ_TYPE_PROVIDER_KEY_DELETE,
    REQ_TYPE_PROVIDER_KEY_CONFIG_GET, REQ_TYPE_PROVIDER_KEY_CONFIG_SET,
    REQ_TYPE_PROVIDER_KEY_LIST, REQ_TYPE_PROVIDER_KEY_TOGGLE, REQ_TYPE_PROVIDER_KEY_WEIGHT_SET,
};
use crate::routing::KeyRotationStrategy;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{key_display_hint, mask_key};

#[derive(Debug, Deserialize)]
pub(super) struct KeyPayload {
    key: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeyTogglePayload {
    key: String,
    active: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeysBatchPayload {
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeysQuery {
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeyStrategyPayload {
    strategy: KeyRotationStrategy,
}

#[derive(Debug, Deserialize)]
pub(super) struct KeyWeightPayload {
    key: String,
    weight: u32,
}

#[derive(Debug, Serialize)]
struct BatchResult {
    key: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProviderKeyFullEntry {
    value: String,
    masked: String,
    active: bool,
    weight: u32,
}

#[derive(Debug, Serialize)]
struct ProviderKeysConfigResponse {
    strategy: KeyRotationStrategy,
    default_strategy: KeyRotationStrategy,
    strategies: Vec<KeyRotationStrategy>,
    keys: Vec<ProviderKeyFullEntry>,
    total: usize,
}

pub async fn add_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    // 鉴权失败也要记录操作日志与请求日志
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_ADD,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }
    app_state
        .providers
        .add_provider_key(
            &provider_name,
            &payload.key,
            &app_state.config.logging.key_log_strategy,
        )
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
            provider: Some(provider_name.clone()),
            details,
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_ADD,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": true })),
    )
        .into_response())
}

pub async fn toggle_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeyTogglePayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_TOGGLE.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/providers/{}/keys/toggle", provider_name),
            REQ_TYPE_PROVIDER_KEY_TOGGLE,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let updated = app_state
        .providers
        .set_provider_key_active(
            &provider_name,
            &payload.key,
            payload.active,
            &app_state.config.logging.key_log_strategy,
        )
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v, "active": payload.active}).to_string());
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_TOGGLE.to_string(),
            provider: Some(provider_name.clone()),
            details,
        })
        .await;

    if updated {
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/providers/{}/keys/toggle", provider_name),
            REQ_TYPE_PROVIDER_KEY_TOGGLE,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            200,
            None,
        )
        .await;
        Ok((
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response())
    } else {
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/providers/{}/keys/toggle", provider_name),
            REQ_TYPE_PROVIDER_KEY_TOGGLE,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            404,
            Some("key not found".into()),
        )
        .await;
        Err(GatewayError::NotFound("key not found".into()))
    }
}

pub async fn add_provider_keys_batch(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeysBatchPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/providers/{}/keys/batch", provider_name),
            REQ_TYPE_PROVIDER_KEY_ADD,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    if payload.keys.is_empty() {
        return Err(GatewayError::Config("keys array cannot be empty".into()));
    }

    let start_time = Utc::now();
    let mut success = 0usize;
    let mut failed = 0usize;
    let mut results = Vec::with_capacity(payload.keys.len());

    for entry in &payload.keys {
        match app_state
            .providers
            .add_provider_key(
                &provider_name,
                entry,
                &app_state.config.logging.key_log_strategy,
            )
            .await
        {
            Ok(_) => {
                success += 1;
                results.push(BatchResult {
                    key: mask_key(entry),
                    status: "ok".into(),
                    message: None,
                });
            }
            Err(err) => {
                failed += 1;
                results.push(BatchResult {
                    key: mask_key(entry),
                    status: "error".into(),
                    message: Some(err.to_string()),
                });
            }
        }
    }

    let detail = serde_json::json!({
        "added": success,
        "failed": failed,
    })
    .to_string();
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(detail),
        })
        .await;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &format!("/providers/{}/keys/batch", provider_name),
        REQ_TYPE_PROVIDER_KEY_ADD,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "added": success,
            "failed": failed,
            "results": results,
        })),
    )
        .into_response())
}

pub async fn delete_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }
    let deleted = app_state
        .providers
        .remove_provider_key(
            &provider_name,
            &payload.key,
            &app_state.config.logging.key_log_strategy,
        )
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
            provider: Some(provider_name.clone()),
            details,
        })
        .await;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            200,
            None,
        )
        .await;
        Ok((
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response())
    } else {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            404,
            Some("key not found".into()),
        )
        .await;
        Err(GatewayError::NotFound("key not found".into()))
    }
}

pub async fn delete_provider_keys_batch(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeysBatchPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys/batch", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    if payload.keys.is_empty() {
        return Err(GatewayError::Config("keys array cannot be empty".into()));
    }

    let start_time = Utc::now();
    let mut removed = 0usize;
    let mut not_found = 0usize;
    let mut results = Vec::with_capacity(payload.keys.len());

    for raw_key in &payload.keys {
        match app_state
            .providers
            .remove_provider_key(
                &provider_name,
                raw_key,
                &app_state.config.logging.key_log_strategy,
            )
            .await
        {
            Ok(true) => {
                removed += 1;
                results.push(BatchResult {
                    key: mask_key(raw_key),
                    status: "removed".into(),
                    message: None,
                });
            }
            Ok(false) => {
                not_found += 1;
                results.push(BatchResult {
                    key: mask_key(raw_key),
                    status: "not_found".into(),
                    message: None,
                });
            }
            Err(err) => {
                not_found += 1;
                results.push(BatchResult {
                    key: mask_key(raw_key),
                    status: "error".into(),
                    message: Some(err.to_string()),
                });
            }
        }
    }

    let detail = serde_json::json!({
        "removed": removed,
        "missing": not_found,
    })
    .to_string();
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(detail),
        })
        .await;

    log_simple_request(
        &app_state,
        start_time,
        "DELETE",
        &format!("/providers/{}/keys/batch", provider_name),
        REQ_TYPE_PROVIDER_KEY_DELETE,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "deleted": removed,
            "missing": not_found,
            "results": results,
        })),
    )
        .into_response())
}

pub async fn list_provider_keys(
    Path(provider_name): Path<String>,
    Query(query): Query<KeysQuery>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_LIST,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }
    let start_time = Utc::now();
    let keys = app_state
        .providers
        .get_provider_keys(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;
    // Always mask in response for safety
    let masked: Vec<String> = keys.iter().map(|k| mask_key(k)).collect();
    let filtered: Vec<String> =
        if let Some(term) = query.q.as_ref().filter(|s| !s.trim().is_empty()) {
            let term_lc = term.to_lowercase();
            masked
                .into_iter()
                .filter(|entry| entry.to_lowercase().contains(&term_lc))
                .collect()
        } else {
            masked
        };

    // audit operation (no keys in details)
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
            provider: Some(provider_name.clone()),
            details: None,
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_LIST,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "keys": filtered })),
    )
        .into_response())
}

pub async fn list_provider_keys_raw(
    Path(provider_name): Path<String>,
    Query(query): Query<KeysQuery>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/providers/{}/keys/raw", provider_name),
            REQ_TYPE_PROVIDER_KEY_LIST,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }
    let start_time = Utc::now();
    let keys = app_state
        .providers
        .list_provider_keys_raw(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let total = keys.len();
    let term = query.q.unwrap_or_default().to_lowercase();
    let entries: Vec<ProviderKeyFullEntry> = keys
        .into_iter()
        .filter(|entry| {
            if term.is_empty() {
                return true;
            }
            let masked = mask_key(&entry.value);
            entry.value.to_lowercase().contains(&term) || masked.to_lowercase().contains(&term)
        })
        .map(|entry| ProviderKeyFullEntry {
            masked: mask_key(&entry.value),
            value: entry.value,
            active: entry.active,
            weight: entry.weight,
        })
        .collect();

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
            provider: Some(provider_name.clone()),
            details: Some("raw".into()),
        })
        .await;

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &format!("/providers/{}/keys/raw", provider_name),
        REQ_TYPE_PROVIDER_KEY_LIST,
        None,
        Some(provider_name.clone()),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({
            "keys": entries,
            "total": total,
        })),
    )
        .into_response())
}

pub async fn get_provider_keys_config(
    Path(provider_name): Path<String>,
    Query(query): Query<KeysQuery>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_CONFIG_GET.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/providers/{}/keys/config", provider_name),
            REQ_TYPE_PROVIDER_KEY_CONFIG_GET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let start_time = Utc::now();
    let strategy = app_state
        .providers
        .get_provider_key_rotation_strategy(&provider_name)
        .await
        .map_err(GatewayError::Db)?;
    let keys = app_state
        .providers
        .list_provider_keys_raw(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let total = keys.len();
    let term = query.q.unwrap_or_default().to_lowercase();
    let entries: Vec<ProviderKeyFullEntry> = keys
        .into_iter()
        .filter(|entry| {
            if term.is_empty() {
                return true;
            }
            let masked = mask_key(&entry.value);
            entry.value.to_lowercase().contains(&term) || masked.to_lowercase().contains(&term)
        })
        .map(|entry| ProviderKeyFullEntry {
            masked: mask_key(&entry.value),
            value: entry.value,
            active: entry.active,
            weight: entry.weight,
        })
        .collect();

    let resp = ProviderKeysConfigResponse {
        strategy,
        default_strategy: KeyRotationStrategy::default(),
        strategies: vec![
            KeyRotationStrategy::Sequential,
            KeyRotationStrategy::Random,
            KeyRotationStrategy::WeightedSequential,
            KeyRotationStrategy::WeightedRandom,
        ],
        keys: entries,
        total,
    };

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_CONFIG_GET.to_string(),
            provider: Some(provider_name.clone()),
            details: None,
        })
        .await;

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &format!("/providers/{}/keys/config", provider_name),
        REQ_TYPE_PROVIDER_KEY_CONFIG_GET,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((axum::http::StatusCode::OK, Json(resp)).into_response())
}

pub async fn put_provider_keys_config(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeyStrategyPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_CONFIG_SET.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/providers/{}/keys/config", provider_name),
            REQ_TYPE_PROVIDER_KEY_CONFIG_SET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let updated = app_state
        .providers
        .set_provider_key_rotation_strategy(&provider_name, payload.strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_CONFIG_SET.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(
                serde_json::json!({
                    "strategy": payload.strategy.as_db_value(),
                })
                .to_string(),
            ),
        })
        .await;

    if updated {
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/providers/{}/keys/config", provider_name),
            REQ_TYPE_PROVIDER_KEY_CONFIG_SET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            200,
            None,
        )
        .await;
        Ok((
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response())
    } else {
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/providers/{}/keys/config", provider_name),
            REQ_TYPE_PROVIDER_KEY_CONFIG_SET,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            404,
            Some("provider not found".into()),
        )
        .await;
        Err(GatewayError::NotFound("provider not found".into()))
    }
}

pub async fn patch_provider_key_weight(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<KeyWeightPayload>,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let start_time = chrono::Utc::now();
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_KEY_WEIGHT_SET.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PATCH",
            &format!("/providers/{}/keys/weight", provider_name),
            REQ_TYPE_PROVIDER_KEY_WEIGHT_SET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }
    if payload.weight < 1 {
        return Err(GatewayError::Config("weight must be >= 1".into()));
    }

    let updated = app_state
        .providers
        .set_provider_key_weight(
            &provider_name,
            &payload.key,
            payload.weight,
            &app_state.config.logging.key_log_strategy,
        )
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| {
        serde_json::json!({
            "key": v,
            "weight": payload.weight,
        })
        .to_string()
    });
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_WEIGHT_SET.to_string(),
            provider: Some(provider_name.clone()),
            details,
        })
        .await;

    if updated {
        log_simple_request(
            &app_state,
            start_time,
            "PATCH",
            &format!("/providers/{}/keys/weight", provider_name),
            REQ_TYPE_PROVIDER_KEY_WEIGHT_SET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            200,
            None,
        )
        .await;
        Ok((
            axum::http::StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response())
    } else {
        log_simple_request(
            &app_state,
            start_time,
            "PATCH",
            &format!("/providers/{}/keys/weight", provider_name),
            REQ_TYPE_PROVIDER_KEY_WEIGHT_SET,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            404,
            Some("key not found".into()),
        )
        .await;
        Err(GatewayError::NotFound("key not found".into()))
    }
}
