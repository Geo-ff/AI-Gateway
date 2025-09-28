use axum::{extract::{Path, State}, response::{IntoResponse, Response}, Json};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_PROVIDER_KEY_ADD, REQ_TYPE_PROVIDER_KEY_DELETE, REQ_TYPE_PROVIDER_KEY_LIST, ProviderOpLog};
use crate::server::request_logging::log_simple_request;
use super::auth::ensure_admin;
use crate::server::AppState;
use crate::server::util::{key_display_hint, mask_key};

#[derive(Debug, Deserialize)]
pub(super) struct KeyPayload { key: String }

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
    if let Err(e) = ensure_admin(&headers, &app_state) {
        let start_time = chrono::Utc::now();
        let _ = app_state.log_store.log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(e.to_string()),
        }).await;
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
        ).await;
        return Err(e);
    }
    if !app_state.providers.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    app_state
        .providers
        .add_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state.log_store.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_KEY_ADD.to_string(),
        provider: Some(provider_name.clone()),
        details,
    }).await;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_ADD,
        None,
        Some(provider_name),
        provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }),
        201,
        None,
    )
    .await;

    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({"status":"ok"}))).into_response())
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
    if let Err(e) = ensure_admin(&headers, &app_state) {
        let start_time = chrono::Utc::now();
        let _ = app_state.log_store.log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(e.to_string()),
        }).await;
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
        ).await;
        return Err(e);
    }
    if !app_state.providers.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    let deleted = app_state
        .providers
        .remove_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state.log_store.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_KEY_DELETE.to_string(),
        provider: Some(provider_name.clone()),
        details,
    }).await;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name),
            provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }),
            200,
            None,
        )
        .await;
        Ok((axum::http::StatusCode::OK, Json(serde_json::json!({"status":"ok"}))).into_response())
    } else {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/keys", provider_name),
            REQ_TYPE_PROVIDER_KEY_DELETE,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }),
            404,
            Some("key not found".into()),
        )
        .await;
        Err(GatewayError::NotFound("key not found".into()))
    }
}

pub async fn list_provider_keys(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, GatewayError> {
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = ensure_admin(&headers, &app_state) {
        let start_time = chrono::Utc::now();
        let _ = app_state.log_store.log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(e.to_string()),
        }).await;
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
        ).await;
        return Err(e);
    }
    if !app_state.providers.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    let start_time = Utc::now();
    let keys = app_state
        .providers
        .get_provider_keys(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;
    // Always mask in response for safety
    let masked: Vec<String> = keys.iter().map(|k| mask_key(k)).collect();

    // audit operation (no keys in details)
    let _ = app_state.log_store.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_KEY_LIST.to_string(),
        provider: Some(provider_name.clone()),
        details: None,
    }).await;
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_LIST,
        None,
        Some(provider_name),
        provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }),
        200,
        None,
    )
    .await;

    Ok((axum::http::StatusCode::OK, Json(serde_json::json!({"keys": masked}))).into_response())
}

// key_display_hint and mask_key are imported from server::util
