use axum::{extract::{Path, Query, State}, http::HeaderMap, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::server::AppState;
use crate::logging::types::ProviderOpLog;
use crate::server::request_logging::log_simple_request;
use chrono::Utc;
use super::auth::ensure_admin;

#[derive(Debug, Deserialize)]
pub struct UpsertModelPricePayload {
    pub provider: String,
    pub model: String,
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
    #[serde(default)]
    pub currency: Option<String>,
}

pub async fn upsert_model_price(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertModelPricePayload>,
) -> Result<axum::http::StatusCode, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = ensure_admin(&headers, &app_state) {
        // audit + request logs on failure
        let _ = app_state.log_store.log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: "model_price_upsert".to_string(),
            provider: Some(payload.provider.clone()),
            details: Some(e.to_string()),
        }).await;
        let code = e.status_code().as_u16();
        log_simple_request(&app_state, start_time, "POST", "/admin/model-prices", "model_price_upsert", Some(payload.model.clone()), Some(payload.provider.clone()), provided_token.as_deref(), code, Some("auth failed".into())).await;
        return Err(e);
    }
    // 校验 provider 存在
    if !app_state.providers.provider_exists(&payload.provider).await.map_err(GatewayError::Db)? {
        let ge = GatewayError::NotFound(format!("provider '{}' not found", payload.provider));
        let code = ge.status_code().as_u16();
        log_simple_request(&app_state, start_time, "POST", "/admin/model-prices", "model_price_upsert", Some(payload.model.clone()), Some(payload.provider.clone()), provided_token.as_deref(), code, Some(ge.to_string())).await;
        let _ = app_state.log_store.log_provider_op(ProviderOpLog { id: None, timestamp: start_time, operation: "model_price_upsert".into(), provider: Some(payload.provider.clone()), details: Some("provider not found".into()) }).await;
        return Err(ge);
    }
    // 校验 model 存在于缓存（按 provider 范围）
    let models = crate::server::model_cache::get_cached_models_for_provider(&app_state, &payload.provider)
        .await
        .map_err(GatewayError::Db)?;
    let exists = models.iter().any(|m| m.id == payload.model);
    if !exists {
        let ge = GatewayError::NotFound(format!("model '{}' not found under provider '{}'", payload.model, payload.provider));
        let code = ge.status_code().as_u16();
        log_simple_request(&app_state, start_time, "POST", "/admin/model-prices", "model_price_upsert", Some(payload.model.clone()), Some(payload.provider.clone()), provided_token.as_deref(), code, Some(ge.to_string())).await;
        let _ = app_state.log_store.log_provider_op(ProviderOpLog { id: None, timestamp: start_time, operation: "model_price_upsert".into(), provider: Some(payload.provider.clone()), details: Some("model not found".into()) }).await;
        return Err(ge);
    }
    app_state
        .log_store
        .upsert_model_price(&payload.provider, &payload.model, payload.prompt_price_per_million, payload.completion_price_per_million, payload.currency.as_deref())
        .await
        .map_err(GatewayError::Db)?;
    // Success logs
    let _ = app_state.log_store.log_provider_op(ProviderOpLog { id: None, timestamp: start_time, operation: "model_price_upsert".into(), provider: Some(payload.provider.clone()), details: Some(serde_json::json!({
        "model": payload.model,
        "prompt_price_per_million": payload.prompt_price_per_million,
        "completion_price_per_million": payload.completion_price_per_million,
        "currency": payload.currency,
    }).to_string()) }).await;
    log_simple_request(&app_state, start_time, "POST", "/admin/model-prices", "model_price_upsert", Some(payload.model.clone()), Some(payload.provider.clone()), provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }), 201, None).await;
    Ok(axum::http::StatusCode::CREATED)
}

#[derive(Debug, Deserialize)]
pub struct ListQuery { pub provider: Option<String> }

pub async fn list_model_prices(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = ensure_admin(&headers, &app_state) {
        let code = e.status_code().as_u16();
        log_simple_request(&app_state, start_time, "GET", "/admin/model-prices", "model_price_list", None, q.provider.clone(), provided_token.as_deref(), code, Some("auth failed".into())).await;
        return Err(e);
    }
    let items = app_state
        .log_store
        .list_model_prices(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;
    let out: Vec<_> = items
        .into_iter()
        .map(|(provider, model, p_pm, c_pm, currency)| serde_json::json!({
            "provider": provider,
            "model": model,
            "prompt_price_per_million": p_pm,
            "completion_price_per_million": c_pm,
            "currency": currency,
        }))
        .collect();
    log_simple_request(&app_state, start_time, "GET", "/admin/model-prices", "model_price_list", None, q.provider.clone(), provided_token.as_deref().map(|tok| if tok == app_state.admin_identity_token { "admin_token" } else { tok }), 200, None).await;
    Ok(Json(out))
}

pub async fn get_model_price(
    Path((provider, model)): Path<(String, String)>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = ensure_admin(&headers, &app_state) {
        let code = e.status_code().as_u16();
        log_simple_request(&app_state, start_time, "GET", &format!("/admin/model-prices/{}/{}", provider, model), "model_price_get", Some(model.clone()), Some(provider.clone()), provided_token.as_deref(), code, Some("auth failed".into())).await;
        return Err(e);
    }
    match app_state.log_store.get_model_price(&provider, &model).await.map_err(GatewayError::Db)? {
        Some((p_pm, c_pm, currency)) => Ok(Json(serde_json::json!({
            "provider": provider,
            "model": model,
            "prompt_price_per_million": p_pm,
            "completion_price_per_million": c_pm,
            "currency": currency,
        })) ),
        None => {
            let ge = GatewayError::NotFound("model price not set".into());
            let code = ge.status_code().as_u16();
            log_simple_request(&app_state, start_time, "GET", &format!("/admin/model-prices/{}/{}", provider, model), "model_price_get", Some(model.clone()), Some(provider.clone()), provided_token.as_deref(), code, Some(ge.to_string())).await;
            Err(ge)
        },
    }
}
