use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Uri},
    response::{IntoResponse, Json, Response},
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use super::auth::{ensure_admin, ensure_client_token};
use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_MODELS_LIST, REQ_TYPE_PROVIDER_MODELS_LIST};
use crate::providers::openai::ModelListResponse;
use crate::server::AppState;
use crate::server::model_cache::{get_cached_models_all, get_cached_models_for_provider};
use crate::server::model_helpers::fetch_provider_models;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};

pub async fn list_models(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Json<ModelListResponse>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    // 鉴权：优先允许已登录管理员身份（Cookie/TUI Session），否则校验 Client Token
    let mut is_admin = false;
    let mut token_for_limits: Option<String> = None;
    if ensure_admin(&headers, &app_state).await.is_ok() {
        is_admin = true;
    } else {
        match ensure_client_token(&headers, &app_state).await {
            Ok(tok) => token_for_limits = Some(tok),
            Err(e) => {
                let path = uri
                    .path_and_query()
                    .map(|pq| pq.as_str().to_string())
                    .unwrap_or_else(|| "/v1/models".to_string());
                let code = e.status_code().as_u16();
                log_simple_request(
                    &app_state,
                    start_time,
                    "GET",
                    &path,
                    REQ_TYPE_MODELS_LIST,
                    None,
                    None,
                    provided_token.as_deref(),
                    code,
                    Some(e.to_string()),
                )
                .await;
                return Err(e);
            }
        }
    }
    let mut cached_models = get_cached_models_all(&app_state).await?;
    // 若令牌有限制，仅返回该令牌允许的模型
    if !is_admin
        && let Some(tok) = token_for_limits.as_deref()
        && let Some(t) = app_state.token_store.get_token(tok).await?
        && let Some(allow) = t.allowed_models.as_ref()
    {
        use std::collections::HashSet;
        let allow_set: HashSet<&str> = allow.iter().map(|s| s.as_str()).collect();
        cached_models.retain(|m| allow_set.contains(m.id.as_str()));
    }
    let path = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/v1/models".to_string());
    let result = Json(ModelListResponse {
        object: "list".to_string(),
        data: cached_models,
    });
    let token_log = token_for_log(provided_token.as_deref());
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &path,
        REQ_TYPE_MODELS_LIST,
        None,
        None,
        token_log,
        200,
        None,
    )
    .await;
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
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let full_path = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| format!("/models/{}", provider_name));

    if ensure_admin(&headers, &app_state).await.is_err()
        && let Err(e) = ensure_client_token(&headers, &app_state).await
    {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &full_path,
            REQ_TYPE_PROVIDER_MODELS_LIST,
            None,
            Some(provider_name.clone()),
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let provider = match app_state
        .providers
        .get_provider(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        Some(p) => p,
        None => {
            let ge = crate::error::GatewayError::NotFound(format!(
                "Provider '{}' not found",
                provider_name
            ));
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &full_path,
                REQ_TYPE_PROVIDER_MODELS_LIST,
                None,
                Some(provider_name.clone()),
                provided_token.as_deref(),
                code,
                Some(format!("Provider '{}' not found", provider_name)),
            )
            .await;
            return Err(ge);
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
            provided_token.as_deref(),
            200,
            None,
        )
        .await;
        let resp = Json(ModelListResponse {
            object: "list".into(),
            data: cached_models,
        });
        return Ok(resp.into_response());
    }

    let api_key = match app_state
        .providers
        .get_provider_keys(&provider_name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?
        .first()
        .cloned()
    {
        Some(k) => k,
        None => {
            let ge: GatewayError =
                crate::routing::load_balancer::BalanceError::NoApiKeysAvailable.into();
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &full_path,
                REQ_TYPE_PROVIDER_MODELS_LIST,
                None,
                Some(provider_name.clone()),
                provided_token.as_deref(),
                code,
                None,
            )
            .await;
            return Err(ge);
        }
    };

    let upstream_models = match fetch_provider_models(&provider, &api_key).await {
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
                provided_token.as_deref(),
                code,
                Some(e.to_string()),
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
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    let resp = Json(ModelListResponse {
        object: "list".into(),
        data: upstream_models,
    })
    .into_response();
    Ok(resp)
}
