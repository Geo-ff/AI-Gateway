use axum::{extract::{Path, State}, response::{IntoResponse, Json, Response}};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_PROVIDER_CACHE_DELETE, REQ_TYPE_PROVIDER_CACHE_UPDATE};
use crate::providers::openai::{Model, ModelListResponse};
use crate::server::model_cache::{cache_models_for_provider, get_cached_models_for_provider};
use crate::server::model_helpers::fetch_provider_models;
use crate::server::request_logging::log_simple_request;
use crate::server::AppState;

#[derive(Debug, Deserialize, Default)]
pub struct CacheUpdatePayload {
    #[serde(default)]
    pub mode: Option<String>, // "all" | "selected"（默认 selected）
    #[serde(default)]
    pub include: Option<Vec<String>>, // 仅 selected 使用
    #[serde(default)]
    pub exclude: Option<Vec<String>>, // 仅 all 使用
    #[serde(default)]
    pub replace: Option<bool>, // selected + include 时覆盖
}

pub async fn update_provider_cache(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<CacheUpdatePayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let path = format!("/models/{}/cache", provider_name);
    let provider = match app_state.config.providers.get(&provider_name) {
        Some(p) => p,
        None => {
            let ge = GatewayError::Config(format!("Provider '{}' not found", provider_name));
            let code = ge.status_code().as_u16();
            log_simple_request(&app_state, start_time, "POST", &path, REQ_TYPE_PROVIDER_CACHE_UPDATE, None, Some(provider_name), code).await;
            return Err(ge);
        }
    };

    let api_key = match provider.api_keys.first().cloned() {
        Some(k) => k,
        None => {
            let ge: GatewayError = crate::routing::load_balancer::BalanceError::NoApiKeysAvailable.into();
            let code = ge.status_code().as_u16();
            log_simple_request(&app_state, start_time, "POST", &path, REQ_TYPE_PROVIDER_CACHE_UPDATE, None, Some(provider_name.clone()), code).await;
            return Err(ge);
        }
    };

    let mode = payload.mode.as_deref().unwrap_or("selected");
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut updated = 0usize;
    let mut filtered = 0usize;

    let prev = crate::server::model_cache::get_cached_models_for_provider(&app_state, &provider_name)
        .await
        .unwrap_or_default();

    match mode {
        "all" => {
            let mut upstream_models = fetch_provider_models(provider, &api_key).await?;
            if let Some(ex) = payload.exclude.as_ref() {
                use std::collections::HashSet;
                let set: HashSet<_> = ex.iter().map(|s| s.as_str()).collect();
                let before = upstream_models.len();
                upstream_models.retain(|m| !set.contains(m.id.as_str()));
                filtered = before - upstream_models.len();
            }
            use std::collections::{HashMap, HashSet};
            let prev_map: HashMap<_, _> = prev.iter().map(|m| (m.id.clone(), (m.object.clone(), m.owned_by.clone(), m.created))).collect();
            let new_ids: HashSet<_> = upstream_models.iter().map(|m| m.id.clone()).collect();
            let old_ids: HashSet<_> = prev_map.keys().cloned().collect();
            added = new_ids.difference(&old_ids).count();
            removed = old_ids.difference(&new_ids).count();
            updated = new_ids.intersection(&old_ids).filter(|id| {
                let old = prev_map.get(*id).unwrap();
                let new = upstream_models.iter().find(|m| &m.id == *id).unwrap();
                old.0 != new.object || old.1 != new.owned_by || old.2 != new.created
            }).count();
            let _ = cache_models_for_provider(&app_state, &provider_name, &upstream_models).await;
        }
        "selected" => {
            let include = payload.include.clone().unwrap_or_default();
            if include.is_empty() {
                let ge = GatewayError::Config("include cannot be empty for mode=selected".to_string());
                let code = ge.status_code().as_u16();
                log_simple_request(&app_state, start_time, "POST", &path, REQ_TYPE_PROVIDER_CACHE_UPDATE, None, Some(provider_name.clone()), code).await;
                return Err(ge);
            }
            let upstream_models = fetch_provider_models(provider, &api_key).await?;
            use std::collections::{HashMap, HashSet};
            let include_ids: HashSet<_> = include.iter().map(|s| s.as_str()).collect();
            let selected: Vec<Model> = upstream_models
                .iter()
                .filter(|m| include_ids.contains(m.id.as_str()))
                .cloned()
                .collect();
            let prev_map: HashMap<_, _> = prev.iter().map(|m| (m.id.clone(), (m.object.clone(), m.owned_by.clone(), m.created))).collect();
            let sel_ids: HashSet<_> = selected.iter().map(|m| m.id.clone()).collect();
            let prev_ids: HashSet<_> = prev_map.keys().cloned().collect();
            let replace = payload.replace.unwrap_or(false);
            if replace {
                added = sel_ids.difference(&prev_ids).count();
                updated = sel_ids.intersection(&prev_ids).filter(|id| {
                    let old = prev_map.get(*id).unwrap();
                    let new = selected.iter().find(|m| &m.id == *id).unwrap();
                    old.0 != new.object || old.1 != new.owned_by || old.2 != new.created
                }).count();
                removed = prev_ids.difference(&sel_ids).count();
                let _ = cache_models_for_provider(&app_state, &provider_name, &selected).await;
            } else {
                added = sel_ids.difference(&prev_ids).count();
                updated = sel_ids.intersection(&prev_ids).filter(|id| {
                    let old = prev_map.get(*id).unwrap();
                    let new = selected.iter().find(|m| &m.id == *id).unwrap();
                    old.0 != new.object || old.1 != new.owned_by || old.2 != new.created
                }).count();
                let _ = crate::server::model_cache::cache_models_for_provider_append(&app_state, &provider_name, &selected).await;
            }
        }
        _ => {
            let ge = GatewayError::Config("invalid mode".to_string());
            let code = ge.status_code().as_u16();
            log_simple_request(&app_state, start_time, "POST", &path, REQ_TYPE_PROVIDER_CACHE_UPDATE, None, Some(provider_name.clone()), code).await;
            return Err(ge);
        }
    }

    let models = get_cached_models_for_provider(&app_state, &provider_name).await?;
    log_simple_request(&app_state, start_time, "POST", &path, REQ_TYPE_PROVIDER_CACHE_UPDATE, None, Some(provider_name.clone()), 200).await;
    let mut resp = Json(ModelListResponse { object: "list".into(), data: models }).into_response();
    use axum::http::header::HeaderValue;
    if let Ok(v) = HeaderValue::from_str(&added.to_string()) { resp.headers_mut().insert("X-Cache-Added", v); }
    if let Ok(v) = HeaderValue::from_str(&removed.to_string()) { resp.headers_mut().insert("X-Cache-Removed", v); }
    if let Ok(v) = HeaderValue::from_str(&updated.to_string()) { resp.headers_mut().insert("X-Cache-Updated", v); }
    if let Ok(v) = HeaderValue::from_str(&filtered.to_string()) { resp.headers_mut().insert("X-Cache-Filtered", v); }
    Ok(resp)
}

#[derive(Debug, Deserialize, Default)]
pub struct CacheDeletePayload {
    #[serde(default)]
    pub ids: Vec<String>,
}

pub async fn delete_provider_cache(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<CacheDeletePayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let path = format!("/models/{}/cache", provider_name);
    if !app_state.config.providers.contains_key(&provider_name) {
        let ge = GatewayError::Config(format!("Provider '{}' not found", provider_name));
        let code = ge.status_code().as_u16();
        log_simple_request(&app_state, start_time, "DELETE", &path, REQ_TYPE_PROVIDER_CACHE_DELETE, None, Some(provider_name), code).await;
        return Err(ge);
    }

    let ids: Vec<String> = payload
        .ids
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let prev = crate::server::model_cache::get_cached_models_for_provider(&app_state, &provider_name)
        .await
        .unwrap_or_default();
    let removed = ids.iter().filter(|id| prev.iter().any(|m| &m.id == *id)).count();
    let _ = crate::server::model_cache::remove_models_for_provider(&app_state, &provider_name, &ids).await;

    let models = get_cached_models_for_provider(&app_state, &provider_name).await?;
    log_simple_request(&app_state, start_time, "DELETE", &path, REQ_TYPE_PROVIDER_CACHE_DELETE, None, Some(provider_name.clone()), 200).await;
    let mut resp = Json(ModelListResponse { object: "list".into(), data: models }).into_response();
    use axum::http::header::HeaderValue;
    if let Ok(v) = HeaderValue::from_str(&"0".to_string()) { resp.headers_mut().insert("X-Cache-Added", v); }
    if let Ok(v) = HeaderValue::from_str(&removed.to_string()) { resp.headers_mut().insert("X-Cache-Removed", v); }
    if let Ok(v) = HeaderValue::from_str(&"0".to_string()) { resp.headers_mut().insert("X-Cache-Updated", v); }
    if let Ok(v) = HeaderValue::from_str(&"0".to_string()) { resp.headers_mut().insert("X-Cache-Filtered", v); }
    Ok(resp)
}
