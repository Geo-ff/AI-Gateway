use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::error::GatewayError;
use crate::logging::types::ProviderOpLog;
use crate::server::AppState;
use crate::server::model_cache::get_cached_models_for_provider;
use crate::server::request_logging::log_simple_request;
use crate::server::util::bearer_token;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub provider: Option<String>,
}

pub async fn list_model_enabled(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/models/enabled",
            "admin_model_enabled_list",
            None,
            q.provider.clone(),
            provided.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    let items = app_state
        .log_store
        .list_model_enabled(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;

    let out: Vec<_> = items
        .into_iter()
        .map(|(provider, model, enabled)| {
            serde_json::json!({
                "provider": provider,
                "model": model,
                "enabled": enabled,
            })
        })
        .collect();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/models/enabled",
        "admin_model_enabled_list",
        None,
        q.provider.clone(),
        provided.as_deref(),
        200,
        None,
    )
    .await;
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
pub struct UpsertModelEnabledPayload {
    pub provider: String,
    pub model: String,
    pub enabled: bool,
}

pub async fn upsert_model_enabled(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertModelEnabledPayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/models/enabled",
            "admin_model_enabled_upsert",
            Some(payload.model.clone()),
            Some(payload.provider.clone()),
            provided.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&payload.provider)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "provider '{}' not found",
            payload.provider
        )));
    }

    let models = get_cached_models_for_provider(&app_state, &payload.provider)
        .await
        .map_err(GatewayError::Db)?;
    if !models.iter().any(|m| m.id == payload.model) {
        return Err(GatewayError::NotFound(format!(
            "model '{}' not found under provider '{}'",
            payload.model, payload.provider
        )));
    }

    app_state
        .log_store
        .upsert_model_enabled(&payload.provider, &payload.model, payload.enabled)
        .await
        .map_err(GatewayError::Db)?;

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: "admin_model_enabled_upsert".into(),
            provider: Some(payload.provider.clone()),
            details: Some(
                serde_json::json!({
                    "model": payload.model,
                    "enabled": payload.enabled,
                })
                .to_string(),
            ),
        })
        .await;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/models/enabled",
        "admin_model_enabled_upsert",
        Some(payload.model.clone()),
        Some(payload.provider.clone()),
        provided.as_deref(),
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
