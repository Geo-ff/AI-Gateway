use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::config::settings::{Provider, ProviderType};
use crate::error::GatewayError;
use crate::logging::types::{
    ProviderOpLog, REQ_TYPE_PROVIDER_CREATE, REQ_TYPE_PROVIDER_DELETE, REQ_TYPE_PROVIDER_GET,
    REQ_TYPE_PROVIDER_ENABLED_SET, REQ_TYPE_PROVIDER_FAVORITE_SET, REQ_TYPE_PROVIDER_LIST,
    REQ_TYPE_PROVIDER_UPDATE,
};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::storage_traits::FavoriteKind;
use crate::server::util::{bearer_token, mask_key, token_for_log};
use chrono::Utc;

#[derive(Debug, Deserialize)]
pub struct ProviderCreatePayload {
    pub name: String,
    pub display_name: Option<String>,
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderUpdatePayload {
    pub display_name: Option<String>,
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderTogglePayload {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ProviderOut {
    pub name: String,
    pub display_name: Option<String>,
    pub api_type: ProviderType,
    pub base_url: String,
    pub api_keys: Vec<String>,
    pub models_endpoint: Option<String>,
    pub enabled: bool,
    pub is_favorite: bool,
    pub cached_models_count: usize,
}

impl ProviderOut {
    fn from_provider(p: Provider, cached_models_count: usize, is_favorite: bool) -> Self {
        Self {
            name: p.name,
            display_name: p.display_name,
            api_type: p.api_type,
            base_url: p.base_url,
            api_keys: p.api_keys.into_iter().map(|k| mask_key(&k)).collect(),
            models_endpoint: p.models_endpoint,
            enabled: p.enabled,
            is_favorite,
            cached_models_count,
        }
    }
}

pub async fn list_providers(
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<ProviderOut>>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);

    // 获取所有缓存模型，按供应商分组统计数量
    let all_cached = app_state
        .model_cache
        .get_cached_models(None)
        .await
        .unwrap_or_default();
    let mut cached_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in &all_cached {
        *cached_counts.entry(m.provider.clone()).or_insert(0) += 1;
    }

    let favorites: std::collections::HashSet<String> = app_state
        .favorites_store
        .list_favorites(FavoriteKind::Provider)
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .collect();

    let providers = app_state
        .providers
        .list_providers_with_keys(&app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .map(|p| {
            let count = cached_counts.get(&p.name).copied().unwrap_or(0);
            let is_favorite = favorites.contains(&p.name);
            ProviderOut::from_provider(p, count, is_favorite)
        })
        .collect();
    // audit log
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_LIST.to_string(),
            provider: None,
            details: None,
        })
        .await;
    // request log
    let token_log = token_for_log(provided_token.as_deref());
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/providers",
        REQ_TYPE_PROVIDER_LIST,
        None,
        None,
        token_log,
        200,
        None,
    )
    .await;
    Ok(Json(providers))
}

pub async fn get_provider(
    Path(name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ProviderOut>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    match app_state
        .providers
        .get_provider(&name)
        .await
        .map_err(GatewayError::Db)?
    {
        Some(mut p) => {
            p.api_keys = app_state
                .providers
                .get_provider_keys(&name, &app_state.config.logging.key_log_strategy)
                .await
                .map_err(GatewayError::Db)?;
            let cached_count = app_state
                .model_cache
                .get_cached_models(Some(&name))
                .await
                .map(|v| v.len())
                .unwrap_or(0);
            let is_favorite = app_state
                .favorites_store
                .is_favorite(FavoriteKind::Provider, &name)
                .await
                .map_err(GatewayError::Db)?;
            let _ = app_state
                .log_store
                .log_provider_op(ProviderOpLog {
                    id: None,
                    timestamp: start_time,
                    operation: REQ_TYPE_PROVIDER_GET.to_string(),
                    provider: Some(name.clone()),
                    details: None,
                })
                .await;
            let token_log = token_for_log(provided_token.as_deref());
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &format!("/providers/{}", name),
                REQ_TYPE_PROVIDER_GET,
                None,
                Some(name),
                token_log,
                200,
                None,
            )
            .await;
            Ok(Json(ProviderOut::from_provider(p, cached_count, is_favorite)))
        }
        None => {
            let token_log = token_for_log(provided_token.as_deref());
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &format!("/providers/{}", name),
                REQ_TYPE_PROVIDER_GET,
                None,
                Some(name.clone()),
                token_log,
                404,
                Some("not found".into()),
            )
            .await;
            Err(GatewayError::NotFound(format!(
                "Provider '{}' not found",
                name
            )))
        }
    }
}

pub async fn create_provider(
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderCreatePayload>,
) -> Result<Json<ProviderOut>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if payload.name.trim().is_empty() {
        return Err(GatewayError::Config("name cannot be empty".into()));
    }
    if app_state
        .providers
        .provider_exists(&payload.name)
        .await
        .map_err(GatewayError::Db)?
    {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/providers",
            REQ_TYPE_PROVIDER_CREATE,
            None,
            Some(payload.name.clone()),
            token_log,
            400,
            Some("already exists".into()),
        )
        .await;
        return Err(GatewayError::Config("provider already exists".into()));
    }
    let p = Provider {
        name: payload.name.clone(),
        display_name: payload.display_name.clone(),
        api_type: payload.api_type,
        base_url: payload.base_url,
        api_keys: Vec::new(),
        models_endpoint: payload.models_endpoint,
        enabled: true,
    };
    let inserted = match app_state.providers.insert_provider(&p).await {
        Ok(v) => v,
        Err(e) => {
            // 失败时记录操作审计与请求日志
            let _ = app_state
                .log_store
                .log_provider_op(ProviderOpLog {
                    id: None,
                    timestamp: start_time,
                    operation: REQ_TYPE_PROVIDER_CREATE.to_string(),
                    provider: Some(p.name.clone()),
                    details: Some(format!("error: {}", e)),
                })
                .await;
            let ge = GatewayError::Db(e);
            let code = ge.status_code().as_u16();
            let token_for_log = provided_token.as_deref();
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/providers",
                REQ_TYPE_PROVIDER_CREATE,
                None,
                Some(p.name.clone()),
                token_for_log,
                code,
                Some("db error".into()),
            )
            .await;
            return Err(ge);
        }
    };
    if !inserted {
        return Err(GatewayError::Config("provider already exists".into()));
    }
    let _ = app_state.log_store.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_CREATE.to_string(),
        provider: Some(p.name.clone()),
        details: Some(serde_json::to_string(&serde_json::json!({
            "display_name": p.display_name,
            "api_type": match p.api_type { ProviderType::OpenAI => "openai", ProviderType::Anthropic => "anthropic", ProviderType::Zhipu => "zhipu" },
            "base_url": p.base_url,
            "models_endpoint": p.models_endpoint
        })).unwrap_or_default()),
    }).await;
    let token_for_log = provided_token.as_deref();
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/providers",
        REQ_TYPE_PROVIDER_CREATE,
        None,
        Some(p.name.clone()),
        token_for_log,
        200,
        None,
    )
    .await;
    let is_favorite = app_state
        .favorites_store
        .is_favorite(FavoriteKind::Provider, &p.name)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(ProviderOut::from_provider(p, 0, is_favorite)))
}

pub async fn update_provider(
    Path(name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderUpdatePayload>,
) -> Result<Json<ProviderOut>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let existed = app_state
        .providers
        .provider_exists(&name)
        .await
        .map_err(GatewayError::Db)?;
    if !existed {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/providers/{}", name),
            REQ_TYPE_PROVIDER_UPDATE,
            None,
            Some(name.clone()),
            token_log,
            404,
            Some("not found".into()),
        )
        .await;
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            name
        )));
    }
    let existing = app_state
        .providers
        .get_provider(&name)
        .await
        .map_err(GatewayError::Db)?;
    let enabled = existing.as_ref().map(|p| p.enabled).unwrap_or(true);
    let display_name = payload
        .display_name
        .clone()
        .or_else(|| existing.as_ref().and_then(|p| p.display_name.clone()));
    let mut p = Provider {
        name: name.clone(),
        display_name,
        api_type: payload.api_type,
        base_url: payload.base_url,
        api_keys: Vec::new(),
        models_endpoint: payload.models_endpoint,
        enabled,
    };
    app_state
        .providers
        .upsert_provider(&p)
        .await
        .map_err(GatewayError::Db)?;
    p.api_keys = app_state
        .providers
        .get_provider_keys(&name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?;
    let _ = app_state.log_store.log_provider_op(ProviderOpLog {
        id: None,
        timestamp: start_time,
        operation: REQ_TYPE_PROVIDER_UPDATE.to_string(),
        provider: Some(p.name.clone()),
        details: Some(serde_json::to_string(&serde_json::json!({
            "display_name": p.display_name,
            "api_type": match p.api_type { ProviderType::OpenAI => "openai", ProviderType::Anthropic => "anthropic", ProviderType::Zhipu => "zhipu" },
            "base_url": p.base_url,
            "models_endpoint": p.models_endpoint
        })).unwrap_or_default()),
    }).await;
    let token_log = token_for_log(provided_token.as_deref());
    log_simple_request(
        &app_state,
        start_time,
        "PUT",
        &format!("/providers/{}", p.name),
        REQ_TYPE_PROVIDER_UPDATE,
        None,
        Some(p.name.clone()),
        token_log,
        200,
        None,
    )
    .await;
    let cached_count = app_state
        .model_cache
        .get_cached_models(Some(&p.name))
        .await
        .map(|v| v.len())
        .unwrap_or(0);
    let is_favorite = app_state
        .favorites_store
        .is_favorite(FavoriteKind::Provider, &p.name)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(ProviderOut::from_provider(p, cached_count, is_favorite)))
}

#[derive(Debug, Deserialize)]
pub struct ProviderFavoritePayload {
    pub favorite: bool,
}

pub async fn set_provider_favorite(
    Path(name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderFavoritePayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let provider = name.trim().to_string();
    let path = format!("/providers/{}/favorite", provider);

    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_FAVORITE_SET,
            None,
            Some(provider.clone()),
            token_log,
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if provider.is_empty() {
        let ge = GatewayError::Config("provider 不能为空".into());
        let code = ge.status_code().as_u16();
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/providers/{provider}/favorite",
            REQ_TYPE_PROVIDER_FAVORITE_SET,
            None,
            None,
            token_log,
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if !app_state
        .providers
        .provider_exists(&provider)
        .await
        .map_err(GatewayError::Db)?
    {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_FAVORITE_SET,
            None,
            Some(provider.clone()),
            token_log,
            404,
            Some("not found".into()),
        )
        .await;
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider
        )));
    }

    app_state
        .favorites_store
        .set_favorite(FavoriteKind::Provider, &provider, payload.favorite)
        .await
        .map_err(GatewayError::Db)?;

    let token_log = token_for_log(provided_token.as_deref());
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &path,
        REQ_TYPE_PROVIDER_FAVORITE_SET,
        None,
        Some(provider.clone()),
        token_log,
        200,
        None,
    )
    .await;
    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "favorite": payload.favorite })),
    )
        .into_response())
}

pub async fn toggle_provider(
    Path(name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderTogglePayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let path = format!("/providers/{}/toggle", name);

    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_ENABLED_SET.to_string(),
                provider: Some(name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_ENABLED_SET,
            None,
            Some(name.clone()),
            token_log,
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&name)
        .await
        .map_err(GatewayError::Db)?
    {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_ENABLED_SET,
            None,
            Some(name.clone()),
            token_log,
            404,
            Some("not found".into()),
        )
        .await;
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            name
        )));
    }

    let updated = app_state
        .providers
        .set_provider_enabled(&name, payload.enabled)
        .await
        .map_err(GatewayError::Db)?;

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_ENABLED_SET.to_string(),
            provider: Some(name.clone()),
            details: Some(
                serde_json::to_string(&serde_json::json!({ "enabled": payload.enabled }))
                    .unwrap_or_default(),
            ),
        })
        .await;

    if updated {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_ENABLED_SET,
            None,
            Some(name.clone()),
            token_log,
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
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_ENABLED_SET,
            None,
            Some(name.clone()),
            token_log,
            404,
            Some("not found".into()),
        )
        .await;
        Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            name
        )))
    }
}

pub async fn delete_provider(
    Path(name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let deleted = app_state
        .providers
        .delete_provider(&name)
        .await
        .map_err(GatewayError::Db)?;
    if deleted {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_DELETE.to_string(),
                provider: Some(name.clone()),
                details: None,
            })
            .await;
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}", name),
            REQ_TYPE_PROVIDER_DELETE,
            None,
            Some(name),
            token_log,
            200,
            None,
        )
        .await;
        Ok(Json(serde_json::json!({ "success": true })))
    } else {
        let token_log = token_for_log(provided_token.as_deref());
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}", name),
            REQ_TYPE_PROVIDER_DELETE,
            None,
            Some(name.clone()),
            token_log,
            404,
            Some("not found".into()),
        )
        .await;
        Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            name
        )))
    }
}
