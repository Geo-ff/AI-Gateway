use axum::{extract::{Path, Query, State}, http::Uri, response::{IntoResponse, Json, Response}};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_MODELS_LIST, REQ_TYPE_PROVIDER_MODELS_LIST};
use crate::providers::openai::ModelListResponse;
use crate::server::model_helpers::fetch_provider_models;
use crate::server::model_cache::{get_cached_models_all, get_cached_models_for_provider};
use crate::server::request_logging::log_simple_request;
use crate::server::AppState;

pub async fn list_models(
    State(app_state): State<Arc<AppState>>,
    uri: Uri,
) -> Result<Json<ModelListResponse>, GatewayError> {
    let start_time = Utc::now();
    let cached_models = get_cached_models_all(&app_state).await?;
    let path = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/v1/models".to_string());
    let result = Json(ModelListResponse { object: "list".to_string(), data: cached_models });
    log_simple_request(&app_state, start_time, "GET", &path, REQ_TYPE_MODELS_LIST, None, None, 200).await;
    Ok(result)
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ProviderModelsQuery {
    #[serde(default)]
    refresh: Option<bool>,
}

pub async fn list_provider_models(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    Query(params): Query<ProviderModelsQuery>,
    uri: Uri,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let full_path = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| format!("/models/{}", provider_name));

    let provider = match app_state.config.providers.get(&provider_name) {
        Some(p) => p,
        None => {
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &full_path,
                REQ_TYPE_PROVIDER_MODELS_LIST,
                None,
                Some(provider_name.clone()),
                crate::error::GatewayError::Config(format!("Provider '{}' not found", provider_name))
                    .status_code()
                    .as_u16(),
            )
            .await;
            return Err(crate::error::GatewayError::Config(format!("Provider '{}' not found", provider_name)));
        }
    };

    // GET 不再执行任何缓存变更。
    // - 无 refresh：仅返回缓存
    // - refresh=true：拉取上游并返回，但不落库

    if params.refresh != Some(true) {
        let cached_models = get_cached_models_for_provider(&app_state, &provider_name).await?;
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &full_path,
            REQ_TYPE_PROVIDER_MODELS_LIST,
            None,
            Some(provider_name.clone()),
            200,
        )
        .await;
        let resp = Json(ModelListResponse { object: "list".into(), data: cached_models });
        return Ok(resp.into_response());
    }

    let api_key = match provider.api_keys.first().cloned() {
        Some(k) => k,
        None => {
            let ge: GatewayError = crate::routing::load_balancer::BalanceError::NoApiKeysAvailable.into();
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &full_path,
                REQ_TYPE_PROVIDER_MODELS_LIST,
                None,
                Some(provider_name.clone()),
                code,
            )
            .await;
            return Err(ge);
        }
    };

    let upstream_models = match fetch_provider_models(provider, &api_key).await {
        Ok(models) => models,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &full_path,
                REQ_TYPE_PROVIDER_MODELS_LIST,
                None,
                Some(provider_name.clone()),
                code,
            )
            .await;
            return Err(e);
        }
    };

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &full_path,
        REQ_TYPE_PROVIDER_MODELS_LIST,
        None,
        Some(provider_name.clone()),
        200,
    )
    .await;

    let resp = Json(ModelListResponse { object: "list".into(), data: upstream_models }).into_response();
    Ok(resp)
}
