use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::settings::{Provider, ProviderType};
use crate::error::GatewayError;
use crate::server::AppState;
use crate::logging::types::{ProviderOpLog, REQ_TYPE_PROVIDER_CREATE, REQ_TYPE_PROVIDER_UPDATE, REQ_TYPE_PROVIDER_DELETE, REQ_TYPE_PROVIDER_GET, REQ_TYPE_PROVIDER_LIST};
use crate::server::request_logging::log_simple_request;
use chrono::Utc;

#[derive(Debug, Deserialize)]
pub struct ProviderCreatePayload {
    pub name: String,
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderUpdatePayload {
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderOut {
    pub name: String,
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
}

impl From<Provider> for ProviderOut {
    fn from(p: Provider) -> Self {
        Self { name: p.name, api_type: p.api_type, base_url: p.base_url, models_endpoint: p.models_endpoint }
    }
}

pub async fn list_providers(State(app_state): State<Arc<AppState>>) -> Result<Json<Vec<ProviderOut>>, GatewayError> {
    let start_time = Utc::now();
    let providers = app_state
        .db
        .list_providers()
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .map(ProviderOut::from)
        .collect();
    // audit log
    let _ = app_state.db.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_LIST.to_string(),
        provider: None,
        details: None,
    }).await;
    // request log
    log_simple_request(&app_state, start_time, "GET", "/providers", REQ_TYPE_PROVIDER_LIST, None, None, 200, None).await;
    Ok(Json(providers))
}

pub async fn get_provider(Path(name): Path<String>, State(app_state): State<Arc<AppState>>) -> Result<Json<ProviderOut>, GatewayError> {
    let start_time = Utc::now();
    match app_state.db.get_provider(&name).await.map_err(GatewayError::Db)? {
        Some(p) => {
            let _ = app_state.db.log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_GET.to_string(),
                provider: Some(name.clone()),
                details: None,
            }).await;
            log_simple_request(&app_state, start_time, "GET", &format!("/providers/{}", name), REQ_TYPE_PROVIDER_GET, None, Some(name), 200, None).await;
            Ok(Json(ProviderOut::from(p)))
        }
        None => {
            log_simple_request(&app_state, start_time, "GET", &format!("/providers/{}", name), REQ_TYPE_PROVIDER_GET, None, Some(name.clone()), 404, Some("not found".into())).await;
            Err(GatewayError::NotFound(format!("Provider '{}' not found", name)))
        }
    }
}

pub async fn create_provider(State(app_state): State<Arc<AppState>>, Json(payload): Json<ProviderCreatePayload>) -> Result<(axum::http::StatusCode, Json<ProviderOut>), GatewayError> {
    let start_time = Utc::now();
    if payload.name.trim().is_empty() {
        return Err(GatewayError::Config("name cannot be empty".into()));
    }
    if app_state.db.provider_exists(&payload.name).await.map_err(GatewayError::Db)? {
        log_simple_request(&app_state, start_time, "POST", "/providers", REQ_TYPE_PROVIDER_CREATE, None, Some(payload.name.clone()), 400, Some("already exists".into())).await;
        return Err(GatewayError::Config("provider already exists".into()));
    }
    let p = Provider {
        name: payload.name,
        api_type: payload.api_type,
        base_url: payload.base_url,
        api_keys: Vec::new(),
        models_endpoint: payload.models_endpoint,
    };
    let inserted = app_state.db.insert_provider(&p).await.map_err(GatewayError::Db)?;
    if !inserted {
        return Err(GatewayError::Config("provider already exists".into()));
    }
    let _ = app_state.db.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_CREATE.to_string(),
        provider: Some(p.name.clone()),
        details: Some(serde_json::to_string(&serde_json::json!({
            "api_type": match p.api_type { ProviderType::OpenAI => "openai", ProviderType::Anthropic => "anthropic", ProviderType::Zhipu => "zhipu" },
            "base_url": p.base_url,
            "models_endpoint": p.models_endpoint
        })).unwrap_or_default()),
    }).await;
    log_simple_request(&app_state, start_time, "POST", "/providers", REQ_TYPE_PROVIDER_CREATE, None, Some(p.name.clone()), 201, None).await;
    Ok((axum::http::StatusCode::CREATED, Json(ProviderOut::from(p))))
}

pub async fn update_provider(Path(name): Path<String>, State(app_state): State<Arc<AppState>>, Json(payload): Json<ProviderUpdatePayload>) -> Result<(axum::http::StatusCode, Json<ProviderOut>), GatewayError> {
    let start_time = Utc::now();
    let existed = app_state.db.provider_exists(&name).await.map_err(GatewayError::Db)?;
    let p = Provider {
        name: name.clone(),
        api_type: payload.api_type,
        base_url: payload.base_url,
        api_keys: Vec::new(),
        models_endpoint: payload.models_endpoint,
    };
    app_state.db.upsert_provider(&p).await.map_err(GatewayError::Db)?;
    let code = if existed { axum::http::StatusCode::OK } else { axum::http::StatusCode::CREATED };
    let _ = app_state.db.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_UPDATE.to_string(),
        provider: Some(p.name.clone()),
        details: Some(serde_json::to_string(&serde_json::json!({
            "api_type": match p.api_type { ProviderType::OpenAI => "openai", ProviderType::Anthropic => "anthropic", ProviderType::Zhipu => "zhipu" },
            "base_url": p.base_url,
            "models_endpoint": p.models_endpoint
        })).unwrap_or_default()),
    }).await;
    log_simple_request(&app_state, start_time, "PUT", &format!("/providers/{}", p.name), REQ_TYPE_PROVIDER_UPDATE, None, Some(p.name.clone()), if existed {200} else {201}, None).await;
    Ok((code, Json(ProviderOut::from(p))))
}

pub async fn delete_provider(Path(name): Path<String>, State(app_state): State<Arc<AppState>>) -> Result<axum::http::StatusCode, GatewayError> {
    let start_time = Utc::now();
    let deleted = app_state.db.delete_provider(&name).await.map_err(GatewayError::Db)?;
    if deleted {
        let _ = app_state.db.log_provider_op(ProviderOpLog { id: None, timestamp: start_time, operation: REQ_TYPE_PROVIDER_DELETE.to_string(), provider: Some(name.clone()), details: None }).await;
        log_simple_request(&app_state, start_time, "DELETE", &format!("/providers/{}", name), REQ_TYPE_PROVIDER_DELETE, None, Some(name), 204, None).await;
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        log_simple_request(&app_state, start_time, "DELETE", &format!("/providers/{}", name), REQ_TYPE_PROVIDER_DELETE, None, Some(name.clone()), 404, Some("not found".into())).await;
        Err(GatewayError::NotFound(format!("Provider '{}' not found", name)))
    }
}
