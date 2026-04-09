use axum::{Json, extract::State};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::auth::require_superadmin;
use crate::config::settings::ProviderType;
use crate::error::GatewayError;
use crate::logging::types::REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST;
use crate::providers::adapters::{ListModelsRequest, adapter_for, unsupported_provider_message};
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
    auth_fingerprint: Option<String>,
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
    t.as_str()
}

fn normalize_base_url(s: &str) -> String {
    s.trim().trim_end_matches('/').to_string()
}

fn fingerprint_api_key(api_key: &str) -> Option<String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    let hex = hex::encode(hasher.finalize());
    Some(format!("sha256:{}", &hex[..16]))
}

pub(super) async fn invalidate_cache_for_provider(provider: &str) {
    let trimmed = provider.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut guard = cache().lock().await;
    guard.retain(|key, _| key.provider.as_deref() != Some(trimmed));
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

    let capabilities = payload.api_type.capabilities();
    if capabilities.requires_models_endpoint && payload.models_endpoint.is_none() {
        return Err(GatewayError::Config(
            "该 api_type 暂不支持自动获取模型列表；请配置 models_endpoint（OpenAI 兼容响应）或手动输入模型"
                .into(),
        ));
    }

    if !capabilities.supports_auto_model_discovery && !capabilities.requires_models_endpoint {
        return Err(GatewayError::Config(
            unsupported_provider_message(payload.api_type).into(),
        ));
    }

    let mut api_key = payload
        .api_key
        .clone()
        .unwrap_or_default()
        .trim()
        .to_string();
    let provider_name = payload
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let inline_api_key = !api_key.is_empty();
    if api_key.is_empty() {
        if let Some(provider_name) = provider_name.as_deref() {
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

    // 若请求携带临时 key，则不缓存（避免在内存中保存明文 key 相关的派生状态）。
    // 若使用已保存的 provider key，则缓存键必须绑定当前实际生效的 key 指纹，
    // 否则 key 更新后会继续命中旧缓存。
    let cacheable = !inline_api_key;
    let cache_key = CacheKey {
        api_type: api_type_key(&payload.api_type),
        base_url: normalize_base_url(&payload.base_url),
        models_endpoint: payload
            .models_endpoint
            .as_ref()
            .map(|s| s.trim().to_string()),
        provider: provider_name.clone(),
        auth_fingerprint: if cacheable {
            fingerprint_api_key(&api_key)
        } else {
            None
        },
    };
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
                provider_name.clone(),
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            return Ok(result);
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

    let adapter = adapter_for(payload.api_type).ok_or_else(|| {
        GatewayError::Config(unsupported_provider_message(payload.api_type).into())
    })?;
    let models = match adapter
        .list_models(ListModelsRequest {
            models_url: &models_url,
            api_key: &api_key,
        })
        .await
    {
        Ok(models) => models,
        Err(ge) => {
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/providers/models/list",
                REQ_TYPE_PROVIDER_MODELS_BASEURL_LIST,
                None,
                provider_name.clone(),
                token_for_log(provided_token.as_deref()),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

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
        provider_name,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_cache_key(auth_fingerprint: Option<String>, provider: Option<&str>) -> CacheKey {
        CacheKey {
            api_type: "openai",
            base_url: "https://example.com/v1".into(),
            models_endpoint: None,
            provider: provider.map(str::to_string),
            auth_fingerprint,
        }
    }

    #[tokio::test]
    async fn cache_key_differs_when_effective_provider_key_changes() {
        let old_key = demo_cache_key(fingerprint_api_key("sk-old"), Some("demo"));
        let new_key = demo_cache_key(fingerprint_api_key("sk-new"), Some("demo"));

        let mut guard = cache().lock().await;
        guard.clear();
        guard.insert(
            old_key.clone(),
            CacheEntry {
                at: Instant::now(),
                models: vec!["old-model".into()],
            },
        );

        assert!(guard.get(&new_key).is_none());
        assert_eq!(
            guard.get(&old_key).map(|entry| entry.models.clone()),
            Some(vec!["old-model".to_string()])
        );
        guard.clear();
    }

    #[tokio::test]
    async fn invalidate_cache_for_provider_removes_stale_models() {
        let demo_key = demo_cache_key(fingerprint_api_key("sk-demo"), Some("demo"));
        let other_key = demo_cache_key(fingerprint_api_key("sk-other"), Some("other"));

        {
            let mut guard = cache().lock().await;
            guard.clear();
            guard.insert(
                demo_key.clone(),
                CacheEntry {
                    at: Instant::now(),
                    models: vec!["demo-model".into()],
                },
            );
            guard.insert(
                other_key.clone(),
                CacheEntry {
                    at: Instant::now(),
                    models: vec!["other-model".into()],
                },
            );
        }

        invalidate_cache_for_provider("demo").await;

        let mut guard = cache().lock().await;
        assert!(guard.get(&demo_key).is_none());
        assert!(guard.get(&other_key).is_some());
        guard.clear();
    }
}
