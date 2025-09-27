use axum::{extract::{Path, State}, response::{IntoResponse, Response}, Json};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_PROVIDER_KEY_ADD, REQ_TYPE_PROVIDER_KEY_DELETE, REQ_TYPE_PROVIDER_KEY_LIST, ProviderOpLog};
use crate::server::request_logging::log_simple_request;
use crate::server::AppState;
use crate::config::settings::{KeyLogStrategy};

#[derive(Debug, Deserialize)]
pub(super) struct KeyPayload { key: String }

pub async fn add_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    if !app_state.db.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    app_state
        .db
        .add_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state.db.log_provider_op(ProviderOpLog {
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
        201,
        None,
    )
    .await;

    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({"status":"ok"}))).into_response())
}

pub async fn delete_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    if !app_state.db.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    let deleted = app_state
        .db
        .remove_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    // provider ops audit log with masked/plain/none display
    let key_hint = key_display_hint(&app_state.config.logging.key_log_strategy, &payload.key);
    let details = key_hint.map(|v| serde_json::json!({"key": v}).to_string());
    let _ = app_state.db.log_provider_op(ProviderOpLog {
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
) -> Result<Response, GatewayError> {
    if !app_state.db.provider_exists(&provider_name).await.map_err(GatewayError::Db)? {
        return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
    }
    let start_time = Utc::now();
    let keys = app_state
        .db
        .get_provider_keys(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;
    // Always mask in response for safety
    let masked: Vec<String> = keys.iter().map(|k| mask_key(k)).collect();

    // audit operation (no keys in details)
    let _ = app_state.db.log_provider_op(ProviderOpLog {
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
        200,
        None,
    )
    .await;

    Ok((axum::http::StatusCode::OK, Json(serde_json::json!({"keys": masked}))).into_response())
}

fn key_display_hint(strategy: &Option<KeyLogStrategy>, key: &str) -> Option<String> {
    match strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 { return "****".to_string(); }
    let (start, end) = (&key[..4], &key[key.len()-4..]);
    format!("{}****{}", start, end)
}
