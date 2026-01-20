use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Uri},
    response::{IntoResponse, Json, Response},
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use super::auth::{ensure_admin, ensure_client_token, require_user};
use crate::error::GatewayError;
use crate::logging::types::{REQ_TYPE_MODELS_LIST, REQ_TYPE_PROVIDER_MODELS_LIST};
use crate::providers::openai::Model;
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
    // 鉴权：优先允许已登录管理员身份（Cookie/TUI Session），否则允许 AccessToken（登录用户），再否则校验 Client Token
    let mut is_admin = false;
    let mut token_for_limits: Option<String> = None;
    if ensure_admin(&headers, &app_state).await.is_ok() {
        is_admin = true;
    } else if require_user(&headers).is_ok() {
        // 登录用户（普通用户/超级管理员均可），仅用于读取可见模型信息
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

    // 过滤掉已禁用供应商的模型
    {
        use std::collections::HashSet;
        let enabled_providers: HashSet<String> = app_state
            .providers
            .list_providers()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| p.name)
            .collect();
        cached_models.retain(|m| {
            // 模型 id 格式为 "{provider}/{model_id}"
            if let Some(slash_pos) = m.id.find('/') {
                let provider = &m.id[..slash_pos];
                enabled_providers.contains(provider)
            } else {
                true // 无前缀的模型保留
            }
        });
    }

    // 若令牌有限制，仅返回该令牌允许的模型（支持白名单/黑名单）
    if !is_admin
        && let Some(tok) = token_for_limits.as_deref()
        && let Some(t) = app_state.token_store.get_token(tok).await?
    {
        if let Some(allow) = t.allowed_models.as_ref() {
            use std::collections::HashSet;
            let allow_set: HashSet<&str> = allow.iter().map(|s| s.as_str()).collect();
            cached_models.retain(|m| allow_set.contains(m.id.as_str()));
        }
        if let Some(deny) = t.model_blacklist.as_ref() {
            use std::collections::HashSet;
            let deny_set: HashSet<&str> = deny.iter().map(|s| s.as_str()).collect();
            cached_models.retain(|m| !deny_set.contains(m.id.as_str()));
        }
    }

    // 过滤掉被管理员禁用的模型（single source: model_settings；未设置则视为 enabled）
    {
        use std::collections::HashSet;
        let disabled: HashSet<String> = app_state
            .log_store
            .list_model_enabled(None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, _, enabled)| !*enabled)
            .map(|(provider, model, _)| format!("{}/{}", provider, model))
            .collect();
        cached_models.retain(|m| !disabled.contains(&m.id));
    }

    // 若该 provider 配置了 redirects，则对外仅暴露 target 模型：
    // - source 将被折叠为最终 target（链式重定向取最终落点）
    // - 若 target 不在缓存列表中，则合成一个 target entry（复用 source 的元信息）
    {
        use std::collections::{HashMap, HashSet};
        fn resolve_redirect_chain(
            map: &HashMap<String, String>,
            source_model: &str,
            max_hops: usize,
        ) -> String {
            let mut current = source_model.to_string();
            let mut seen = HashSet::<String>::new();
            for _ in 0..max_hops {
                if !seen.insert(current.clone()) {
                    break;
                }
                match map.get(&current) {
                    Some(next) if next != &current => current = next.clone(),
                    _ => break,
                }
            }
            current
        }

        let original_ids: HashSet<String> = cached_models.iter().map(|m| m.id.clone()).collect();
        let mut out = Vec::with_capacity(cached_models.len());
        let mut seen = HashSet::<String>::new();
        let mut redirects_cache = HashMap::<String, HashMap<String, String>>::new();

        for m in cached_models.into_iter() {
            let (provider, model_id) = match m.id.split_once('/') {
                Some((p, mid)) => (p.to_string(), mid.to_string()),
                None => {
                    if seen.insert(m.id.clone()) {
                        out.push(m);
                    }
                    continue;
                }
            };
            let map = if redirects_cache.contains_key(&provider) {
                redirects_cache.get(&provider).cloned().unwrap_or_default()
            } else {
                let pairs = app_state
                    .providers
                    .list_model_redirects(&provider)
                    .await
                    .map_err(GatewayError::Db)?;
                let map: HashMap<String, String> = pairs.into_iter().collect();
                redirects_cache.insert(provider.clone(), map.clone());
                map
            };

            if map.is_empty() {
                if seen.insert(m.id.clone()) {
                    out.push(m);
                }
                continue;
            }

            let resolved = resolve_redirect_chain(&map, &model_id, 16);
            if resolved == model_id {
                if seen.insert(m.id.clone()) {
                    out.push(m);
                }
                continue;
            }

            let target_id = format!("{}/{}", provider, resolved);
            if original_ids.contains(&target_id) {
                continue;
            }
            if !seen.insert(target_id.clone()) {
                continue;
            }
            out.push(Model {
                id: target_id,
                object: m.object,
                created: m.created,
                owned_by: m.owned_by,
            });
        }

        cached_models = out;
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

    if ensure_admin(&headers, &app_state).await.is_err() && require_user(&headers).is_err() {
        if let Err(e) = ensure_client_token(&headers, &app_state).await {
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
        let disabled = app_state
            .log_store
            .list_model_enabled(Some(&provider_name))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, _, enabled)| !*enabled)
            .map(|(_, model, _)| model)
            .collect::<std::collections::HashSet<_>>();
        let mut cached_models: Vec<_> = cached_models
            .into_iter()
            .filter(|m| !disabled.contains(&m.id))
            .collect();

        // 若配置了 redirects，则对外仅暴露 target 模型（source 折叠为最终 target）
        {
            use std::collections::{HashMap, HashSet};
            let pairs = app_state
                .providers
                .list_model_redirects(&provider_name)
                .await
                .map_err(GatewayError::Db)?;
            if !pairs.is_empty() {
                let map: HashMap<String, String> = pairs.into_iter().collect();
                fn resolve_redirect_chain(
                    map: &HashMap<String, String>,
                    source_model: &str,
                    max_hops: usize,
                ) -> String {
                    let mut current = source_model.to_string();
                    let mut seen = HashSet::<String>::new();
                    for _ in 0..max_hops {
                        if !seen.insert(current.clone()) {
                            break;
                        }
                        match map.get(&current) {
                            Some(next) if next != &current => current = next.clone(),
                            _ => break,
                        }
                    }
                    current
                }

                let original_ids: HashSet<String> =
                    cached_models.iter().map(|m| m.id.clone()).collect();
                let mut out = Vec::with_capacity(cached_models.len());
                let mut seen = HashSet::<String>::new();
                for m in cached_models.into_iter() {
                    let resolved = resolve_redirect_chain(&map, &m.id, 16);
                    if resolved == m.id {
                        if seen.insert(m.id.clone()) {
                            out.push(m);
                        }
                        continue;
                    }
                    if original_ids.contains(&resolved) {
                        continue;
                    }
                    if !seen.insert(resolved.clone()) {
                        continue;
                    }
                    out.push(Model {
                        id: resolved,
                        object: m.object,
                        created: m.created,
                        owned_by: m.owned_by,
                    });
                }
                cached_models = out;
            }
        }
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
