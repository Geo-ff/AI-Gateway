use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Uri},
    response::{IntoResponse, Json, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone)]
struct CachedModelInfo {
    full_id: String,
    provider: String,
    model_id: String,
    object: String,
    created: u64,
    owned_by: String,
    cached_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct MyModelTokenOut {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct MyModelOut {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub model_type: Option<String>,
    pub model_types: Option<Vec<String>>,
    pub provider: String,
    pub provider_id: String,
    pub provider_enabled: bool,
    pub upstream_endpoint_type: String,
    pub input_price: Option<f64>,
    pub output_price: Option<f64>,
    pub redirect_name: String,
    pub status: String,
    pub created_at: String,
    pub is_favorite: bool,
    pub tokens: Vec<MyModelTokenOut>,
}

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

pub async fn list_my_models(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Json<Vec<MyModelOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let path = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/me/models".to_string());

    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &path,
                "me_models_list",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let tokens = app_state
        .token_store
        .list_tokens_by_user(&claims.sub)
        .await?;
    let now = Utc::now();
    let mut usable_tokens = Vec::new();
    for t in tokens {
        if !t.enabled {
            continue;
        }
        if let Some(exp) = t.expires_at.as_ref()
            && now > *exp
        {
            continue;
        }
        if let Some(max_amount) = t.max_amount {
            if let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&t.token)
                .await
            {
                if spent >= max_amount {
                    continue;
                }
            }
        }
        usable_tokens.push(t);
    }

    if usable_tokens.is_empty() {
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &path,
            "me_models_list",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(Json(vec![]));
    }

    // Providers (for display name, enabled, api_type)
    let providers = app_state
        .providers
        .list_providers()
        .await
        .unwrap_or_default();
    let providers_by_id: std::collections::HashMap<String, crate::config::settings::Provider> =
        providers.into_iter().map(|p| (p.name.clone(), p)).collect();

    // Cached models (base universe)
    let cached = app_state.model_cache.get_cached_models(None).await?;
    let mut base_models: Vec<CachedModelInfo> = cached
        .into_iter()
        .map(|m| CachedModelInfo {
            full_id: format!("{}/{}", m.provider, m.id),
            provider: m.provider,
            model_id: m.id,
            object: m.object,
            created: m.created,
            owned_by: m.owned_by,
            cached_at: m.cached_at,
        })
        .collect();

    // Filter out disabled providers (when provider info is available)
    if !providers_by_id.is_empty() {
        base_models.retain(|m| {
            providers_by_id
                .get(&m.provider)
                .map(|p| p.enabled)
                .unwrap_or(true)
        });
    }

    // Filter out disabled models (single source: model_settings; unset => enabled)
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
        base_models.retain(|m| !disabled.contains(&m.full_id));
    }

    // Prices (optional; missing => null)
    let mut price_by_key = std::collections::HashMap::<String, (f64, f64, Option<String>)>::new();
    if let Ok(items) = app_state.log_store.list_model_prices(None).await {
        for (provider, model, p_pm, c_pm, _currency, model_type) in items {
            price_by_key.insert(format!("{}:{}", provider, model), (p_pm, c_pm, model_type));
        }
    }

    // Build union of token-visible models (supports allowlist/denylist + redirects folding).
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

    let mut redirects_cache = HashMap::<String, HashMap<String, String>>::new();
    let mut union = HashMap::<String, CachedModelInfo>::new();
    let mut tokens_by_model = HashMap::<String, HashMap<String, String>>::new();

    for token in usable_tokens.into_iter() {
        let token_out = MyModelTokenOut {
            id: token.id.clone(),
            name: token.name.clone(),
        };
        let mut models_for_token = base_models.clone();

        if let Some(allow) = token.allowed_models.as_ref() {
            let allow_set: HashSet<&str> = allow.iter().map(|s| s.as_str()).collect();
            models_for_token.retain(|m| allow_set.contains(m.full_id.as_str()));
        }
        if let Some(deny) = token.model_blacklist.as_ref() {
            let deny_set: HashSet<&str> = deny.iter().map(|s| s.as_str()).collect();
            models_for_token.retain(|m| !deny_set.contains(m.full_id.as_str()));
        }

        let original_ids: HashSet<String> =
            models_for_token.iter().map(|m| m.full_id.clone()).collect();
        let mut out = Vec::with_capacity(models_for_token.len());
        let mut seen = HashSet::<String>::new();

        for m in models_for_token.into_iter() {
            let map = if redirects_cache.contains_key(&m.provider) {
                redirects_cache
                    .get(&m.provider)
                    .cloned()
                    .unwrap_or_default()
            } else {
                let pairs = app_state
                    .providers
                    .list_model_redirects(&m.provider)
                    .await
                    .map_err(GatewayError::Db)?;
                let map: HashMap<String, String> = pairs.into_iter().collect();
                redirects_cache.insert(m.provider.clone(), map.clone());
                map
            };

            if map.is_empty() {
                if seen.insert(m.full_id.clone()) {
                    out.push(m);
                }
                continue;
            }

            let resolved = resolve_redirect_chain(&map, &m.model_id, 16);
            if resolved == m.model_id {
                if seen.insert(m.full_id.clone()) {
                    out.push(m);
                }
                continue;
            }

            let target_id = format!("{}/{}", m.provider, resolved);
            if original_ids.contains(&target_id) {
                continue;
            }
            if !seen.insert(target_id.clone()) {
                continue;
            }
            out.push(CachedModelInfo {
                full_id: target_id,
                provider: m.provider,
                model_id: resolved,
                object: m.object,
                created: m.created,
                owned_by: m.owned_by,
                cached_at: m.cached_at,
            });
        }

        for m in out.into_iter() {
            let model_id = m.full_id.clone();
            union.entry(model_id.clone()).or_insert(m);
            tokens_by_model
                .entry(model_id)
                .or_default()
                .insert(token_out.id.clone(), token_out.name.clone());
        }
    }

    let mut out: Vec<MyModelOut> = Vec::with_capacity(union.len());
    for m in union.into_values() {
        let provider = providers_by_id.get(&m.provider);
        let (input_price, output_price, model_type_raw) = {
            let direct = price_by_key.get(&format!("{}:{}", m.provider, m.model_id));
            let mut picked = direct.cloned();

            if picked.is_none() {
                if let Some(map) = redirects_cache.get(&m.provider) {
                    let mut sources: Vec<&String> = map.keys().collect();
                    sources.sort();
                    for source in sources {
                        let resolved = resolve_redirect_chain(map, source, 16);
                        if resolved != m.model_id {
                            continue;
                        }
                        if let Some(p) = price_by_key.get(&format!("{}:{}", m.provider, source)) {
                            picked = Some(p.clone());
                            break;
                        }
                    }
                }
            }

            picked
                .map(|(p, c, mt)| (Some(p), Some(c), mt))
                .unwrap_or((None, None, None))
        };
        let (model_type, model_types) =
            crate::server::model_types::model_types_for_response(model_type_raw.as_deref());

        let mut tokens: Vec<MyModelTokenOut> = tokens_by_model
            .remove(&m.full_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(id, name)| MyModelTokenOut { id, name })
            .collect();
        tokens.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));

        out.push(MyModelOut {
            id: m.full_id.clone(),
            name: m.model_id.clone(),
            model_id: m.model_id,
            model_type,
            model_types,
            provider: provider
                .and_then(|p| p.display_name.clone())
                .unwrap_or_else(|| m.provider.clone()),
            provider_id: m.provider.clone(),
            provider_enabled: provider.map(|p| p.enabled).unwrap_or(true),
            upstream_endpoint_type: provider
                .map(|p| {
                    match p.api_type {
                        crate::config::settings::ProviderType::OpenAI => "openai",
                        crate::config::settings::ProviderType::Anthropic => "anthropic",
                        crate::config::settings::ProviderType::Zhipu => "zhipu",
                        crate::config::settings::ProviderType::Doubao => "doubao",
                    }
                    .to_string()
                })
                .unwrap_or_else(|| "unknown".to_string()),
            input_price,
            output_price,
            redirect_name: String::new(),
            status: "enabled".to_string(),
            created_at: m.cached_at.to_rfc3339(),
            is_favorite: false,
            tokens,
        });
    }

    out.sort_by(|a, b| {
        a.provider_id
            .cmp(&b.provider_id)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &path,
        "me_models_list",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;

    Ok(Json(out))
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
