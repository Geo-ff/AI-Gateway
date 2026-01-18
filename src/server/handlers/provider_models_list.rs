use axum::{Json, extract::State};
use chrono::Utc;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::auth::require_superadmin;
use crate::config::settings::ProviderType;
use crate::error::GatewayError;
use crate::logging::types::REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST;
use crate::providers::openai::ModelListResponse;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::ssrf::{join_models_url, validate_outbound_base_url};
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Deserialize)]
pub struct ProviderModelsListPayload {
    pub api_type: ProviderType,
    pub base_url: String,
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// 临时 Key：仅用于“查看可用模型”，后端不会保存，也不会写入日志。
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderModelsListResponse {
    pub models: Vec<String>,
    pub raw_count: usize,
    pub cached: bool,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct CacheKey {
    api_type: &'static str,
    base_url: String,
    models_endpoint: Option<String>,
    provider: Option<String>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    at: Instant,
    models: Vec<String>,
}

static MODELS_LIST_CACHE: OnceLock<Mutex<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<CacheKey, CacheEntry>> {
    MODELS_LIST_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn api_type_key(t: &ProviderType) -> &'static str {
    match t {
        ProviderType::OpenAI => "openai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Zhipu => "zhipu",
    }
}

fn normalize_base_url(s: &str) -> String {
    s.trim().trim_end_matches('/').to_string()
}

pub async fn list_models_by_base_url(
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderModelsListPayload>,
) -> Result<Json<ProviderModelsListResponse>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);

    if payload.base_url.trim().is_empty() {
        return Err(GatewayError::Config("base_url 不能为空".into()));
    }

    // SSRF：先校验 base_url；若 models_endpoint 是完整 URL，后续还会再校验一次
    let base_url = validate_outbound_base_url(&payload.base_url).await?;

    if !matches!(payload.api_type, ProviderType::OpenAI) && payload.models_endpoint.is_none() {
        return Err(GatewayError::Config(
            "该 api_type 暂不支持自动获取模型列表；请配置 models_endpoint（OpenAI 兼容响应）或手动输入模型"
                .into(),
        ));
    }

    let cache_key = CacheKey {
        api_type: api_type_key(&payload.api_type),
        base_url: normalize_base_url(&payload.base_url),
        models_endpoint: payload
            .models_endpoint
            .as_ref()
            .map(|s| s.trim().to_string()),
        provider: payload.provider.clone().map(|s| s.trim().to_string()),
    };

    // 若请求携带临时 key，则不缓存（避免在内存中保存明文 key 相关的派生状态）
    let cacheable = payload.api_key.as_deref().map(|s| !s.trim().is_empty()) != Some(true);
    let ttl = Duration::from_secs(120);

    if cacheable {
        if let Some(hit) = {
            let guard = cache().lock().await;
            guard
                .get(&cache_key)
                .filter(|e| e.at.elapsed() <= ttl)
                .cloned()
        } {
            let result = Json(ProviderModelsListResponse {
                raw_count: hit.models.len(),
                models: hit.models,
                cached: true,
            });
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/providers/models/list",
                REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST,
                None,
                payload.provider.clone(),
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            return Ok(result);
        }
    }

    let mut api_key = payload
        .api_key
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    if api_key.is_empty() {
        if let Some(provider_name) = payload
            .provider
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            api_key = app_state
                .providers
                .get_provider_keys(provider_name, &app_state.config.logging.key_log_strategy)
                .await
                .map_err(GatewayError::Db)?
                .into_iter()
                .next()
                .unwrap_or_default();
            if api_key.is_empty() {
                return Err(GatewayError::Config(
                    "该渠道未配置可用 Key，请先添加并启用 Key 后再获取模型列表".into(),
                ));
            }
        }
    }

    let mut models_url = join_models_url(&base_url, payload.models_endpoint.as_deref())?;
    // 若 models_endpoint 为完整 URL，需再次做 SSRF 校验（host 可能不同）
    if payload
        .models_endpoint
        .as_deref()
        .is_some_and(|ep| ep.trim().starts_with("http://") || ep.trim().starts_with("https://"))
    {
        models_url = validate_outbound_base_url(models_url.as_str()).await?;
    }

    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(12))
        .build()?;

    let mut req = client
        .get(models_url.as_str())
        .header("Accept", "application/json");
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let resp = req.send().await?;
    let status = resp.status();
    let bytes = resp.bytes().await?;

    if !status.is_success() {
        let snippet = String::from_utf8_lossy(&bytes);
        let snippet = snippet.trim();
        let snippet = if snippet.len() > 240 {
            &snippet[..240]
        } else {
            snippet
        };
        let msg = match status.as_u16() {
            401 | 403 => "上游鉴权失败，请检查 Key（或先添加/启用 Key）".to_string(),
            404 => "上游未找到模型列表接口（404），该上游可能不支持自动获取模型列表；可配置 models_endpoint 或手动输入模型".to_string(),
            _ => {
                if snippet.is_empty() {
                    format!("上游返回错误（{}）", status.as_u16())
                } else {
                    format!("上游返回错误（{}）：{}", status.as_u16(), snippet)
                }
            }
        };
        let ge = if matches!(status.as_u16(), 401 | 403) {
            GatewayError::Unauthorized(msg)
        } else {
            GatewayError::Config(msg)
        };
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/providers/models/list",
            REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST,
            None,
            payload.provider.clone(),
            token_for_log(provided_token.as_deref()),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let parsed: ModelListResponse = serde_json::from_slice(&bytes)
        .map_err(|_| GatewayError::Config("解析上游模型列表失败（非 OpenAI 兼容响应）".into()))?;
    let mut models: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
    models.sort();
    models.dedup();

    if cacheable {
        let mut guard = cache().lock().await;
        guard.insert(
            cache_key,
            CacheEntry {
                at: Instant::now(),
                models: models.clone(),
            },
        );
    }

    let result = Json(ProviderModelsListResponse {
        raw_count: models.len(),
        models,
        cached: false,
    });
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/providers/models/list",
        REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST,
        None,
        payload.provider.clone(),
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(result)
}
