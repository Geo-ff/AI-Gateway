use axum::{extract::{Path, State}, response::{IntoResponse, Response}, Json};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_PROVIDER_KEY_ADD, REQ_TYPE_PROVIDER_KEY_DELETE};
use crate::server::request_logging::log_simple_request;
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct KeyPayload { key: String }

pub async fn add_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    if !app_state.config.providers.contains_key(&provider_name) {
        return Err(GatewayError::Config(format!("Provider '{}' not found", provider_name)));
    }
    app_state
        .db
        .add_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_ADD,
        None,
        Some(provider_name),
        201,
    )
    .await;

    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({"status":"ok"}))).into_response())
}

pub async fn delete_provider_key(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<KeyPayload>,
) -> Result<Response, GatewayError> {
    if !app_state.config.providers.contains_key(&provider_name) {
        return Err(GatewayError::Config(format!("Provider '{}' not found", provider_name)));
    }
    app_state
        .db
        .remove_provider_key(&provider_name, &payload.key, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;

    let start_time = Utc::now();
    log_simple_request(
        &app_state,
        start_time,
        "DELETE",
        &format!("/providers/{}/keys", provider_name),
        REQ_TYPE_PROVIDER_KEY_DELETE,
        None,
        Some(provider_name),
        200,
    )
    .await;

    Ok((axum::http::StatusCode::OK, Json(serde_json::json!({"status":"ok"}))).into_response())
}
