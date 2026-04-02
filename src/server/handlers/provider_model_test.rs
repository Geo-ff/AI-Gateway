use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use super::auth::require_superadmin;
use crate::config::settings::{ProviderConfig, ProviderType, deserialize_default_on_null};
use crate::error::GatewayError;
use crate::logging::types::REQ_TYPE_PROVIDER_MODEL_TEST;
use crate::providers::adapters::{
    ConnectionTestRequest, ListModelsRequest, adapter_for, unsupported_provider_message,
};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::ssrf::{join_models_url, validate_outbound_base_url};
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Deserialize)]
pub struct ProviderModelTestPayload {
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct DraftProviderModelTestPayload {
    pub api_type: ProviderType,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub provider_config: ProviderConfig,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderModelTestResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

fn trim_to_option(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

fn provider_uses_inline_credentials(
    provider_type: ProviderType,
    provider_config: &ProviderConfig,
) -> bool {
    match provider_type {
        ProviderType::AwsClaude => provider_config.has_aws_claude_credentials(),
        ProviderType::VertexAI => provider_config.has_vertex_ai_credentials(),
        _ => false,
    }
}

fn failure_response(
    error_type: impl Into<String>,
    error_message: impl Into<String>,
) -> ProviderModelTestResponse {
    ProviderModelTestResponse {
        success: false,
        latency: None,
        error_type: Some(error_type.into()),
        error_message: Some(error_message.into()),
    }
}

fn unsupported_provider_response(provider_type: ProviderType) -> ProviderModelTestResponse {
    failure_response(
        "unsupported_provider",
        unsupported_provider_message(provider_type),
    )
}

fn map_model_discovery_error(err: GatewayError) -> (String, Option<String>) {
    match err {
        GatewayError::Unauthorized(message) | GatewayError::Forbidden(message) => {
            ("authentication_failed".into(), Some(message))
        }
        GatewayError::Config(message) => {
            let normalized = message.to_lowercase();
            let error_type = if normalized.contains("404")
                || normalized.contains("路径")
                || normalized.contains("url")
                || normalized.contains("models_endpoint 不是合法的")
                || normalized.contains("models_endpoint 需要以")
                || normalized.contains("域名解析失败")
                || normalized.contains("不允许指向")
            {
                "invalid_path"
            } else if normalized.contains("models_endpoint")
                || normalized.contains("api_key")
                || normalized.contains("配置")
                || normalized.contains("不能为空")
                || normalized.contains("required")
            {
                "configuration_required"
            } else {
                "other"
            };
            (error_type.into(), Some(message))
        }
        GatewayError::Http(err) => {
            let error_type = if err.is_timeout() { "timeout" } else { "other" };
            (error_type.into(), Some(err.to_string()))
        }
        other => ("other".into(), Some(other.to_string())),
    }
}

async fn resolve_models_url(
    base_url: &reqwest::Url,
    models_endpoint: Option<&str>,
) -> Result<reqwest::Url, (String, Option<String>)> {
    let mut models_url = join_models_url(base_url, models_endpoint)
        .map_err(|err| ("invalid_path".into(), Some(err.to_string())))?;

    if models_endpoint
        .is_some_and(|endpoint| endpoint.starts_with("http://") || endpoint.starts_with("https://"))
    {
        models_url = validate_outbound_base_url(models_url.as_str())
            .await
            .map_err(|err| ("invalid_path".into(), Some(err.to_string())))?;
    }

    Ok(models_url)
}

async fn resolve_test_model(
    provider_type: ProviderType,
    base_url: &reqwest::Url,
    models_endpoint: Option<&str>,
    api_key: &str,
    _provider_config: &ProviderConfig,
    model: Option<&str>,
) -> Result<String, (String, Option<String>)> {
    if let Some(model) = trim_to_option(model) {
        return Ok(model.to_string());
    }

    let capabilities = provider_type.capabilities();
    if capabilities.requires_models_endpoint && trim_to_option(models_endpoint).is_none() {
        return Err((
            "configuration_required".into(),
            Some("当前端点需要先配置 Models Endpoint，才能自动发现测试模型。".into()),
        ));
    }

    if !capabilities.supports_auto_model_discovery && !capabilities.supports_models_endpoint {
        return Err((
            "configuration_required".into(),
            Some("当前端点不支持自动发现测试模型，请先手动填写模型后再测试。".into()),
        ));
    }

    if api_key.trim().is_empty() {
        return Err((
            "configuration_required".into(),
            Some("model 为空时需要提供 api_key，以便自动发现可用模型。".into()),
        ));
    }

    let models_url = resolve_models_url(base_url, trim_to_option(models_endpoint)).await?;
    let adapter = adapter_for(provider_type).ok_or_else(|| {
        (
            "unsupported_provider".into(),
            Some(unsupported_provider_message(provider_type)),
        )
    })?;

    let model = adapter
        .list_models(ListModelsRequest {
            models_url: &models_url,
            api_key,
        })
        .await
        .map_err(map_model_discovery_error)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            (
                "model_not_found".into(),
                Some("未发现可用模型，请手动填写模型后再测试。".into()),
            )
        })?;

    Ok(model)
}

async fn send_test_request(
    provider_type: ProviderType,
    base_url: &reqwest::Url,
    api_key: &str,
    provider_config: &ProviderConfig,
    model: &str,
    stream: bool,
) -> Result<(), (String, Option<String>)> {
    let adapter = adapter_for(provider_type).ok_or_else(|| {
        (
            "other".into(),
            Some(unsupported_provider_message(provider_type)),
        )
    })?;
    adapter
        .test_connection(ConnectionTestRequest {
            base_url,
            api_key,
            model,
            stream,
            provider_config,
        })
        .await
}

async fn execute_connection_test(
    provider_type: ProviderType,
    base_url: &reqwest::Url,
    api_key: &str,
    provider_config: &ProviderConfig,
    model: &str,
) -> ProviderModelTestResponse {
    let t0 = Instant::now();
    let mut outcome = send_test_request(
        provider_type,
        base_url,
        api_key,
        provider_config,
        model,
        false,
    )
    .await;

    if outcome.is_err()
        && adapter_for(provider_type)
            .map(|adapter| adapter.supports_stream_retry())
            .unwrap_or(false)
        && let Err((_, Some(detail))) = &outcome
    {
        let lower = detail.to_lowercase();
        if lower.contains("bad_response_body") || lower.contains("bad_response_status_code") {
            outcome = send_test_request(
                provider_type,
                base_url,
                api_key,
                provider_config,
                model,
                true,
            )
            .await;
        }
    }

    let latency = t0.elapsed().as_secs_f64();
    match outcome {
        Ok(()) => ProviderModelTestResponse {
            success: true,
            latency: Some(latency),
            error_type: None,
            error_message: None,
        },
        Err((error_type, error_message)) => ProviderModelTestResponse {
            success: false,
            latency: Some(latency),
            error_type: Some(error_type),
            error_message,
        },
    }
}

pub async fn test_provider_model(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<ProviderModelTestPayload>,
) -> Result<Json<ProviderModelTestResponse>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;

    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let path = format!("/providers/{}/models/test", provider_name);

    let provider = match app_state
        .providers
        .get_provider(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        Some(p) => p,
        None => {
            let resp = Json(ProviderModelTestResponse {
                success: false,
                latency: None,
                error_type: Some("other".into()),
                error_message: Some(format!("Provider '{}' not found", provider_name)),
            });
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                &path,
                REQ_TYPE_PROVIDER_MODEL_TEST,
                Some(payload.model.clone()),
                Some(provider_name),
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            return Ok(resp);
        }
    };

    if !provider.enabled {
        let resp = Json(ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some("other".into()),
            error_message: Some("provider is disabled".into()),
        });
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            Some(payload.model.clone()),
            Some(provider_name),
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }

    fn resolve_redirect_chain(
        map: &std::collections::HashMap<String, String>,
        source_model: &str,
        max_hops: usize,
    ) -> String {
        use std::collections::HashSet;
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

    let input_model = payload.model.trim();
    if input_model.is_empty() {
        return Ok(Json(ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some("model_not_found".into()),
            error_message: Some("model cannot be empty".into()),
        }));
    }

    // Apply provider-scoped model redirects, but DO NOT parse/strip prefixes here:
    // this endpoint already pins a provider, so the model string should be sent as-is.
    let redirected_model = app_state
        .providers
        .list_model_redirects(&provider.name)
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    let upstream_model = if redirected_model.is_empty() {
        input_model.to_string()
    } else {
        resolve_redirect_chain(&redirected_model, input_model, 16)
    };

    // SSRF hardening: validate outbound base_url before request.
    let base_url = match validate_outbound_base_url(&provider.base_url).await {
        Ok(ok) => ok,
        Err(e) => {
            let resp = Json(ProviderModelTestResponse {
                success: false,
                latency: None,
                error_type: Some("invalid_path".into()),
                error_message: Some(e.to_string()),
            });
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                &path,
                REQ_TYPE_PROVIDER_MODEL_TEST,
                Some(payload.model.clone()),
                Some(provider_name),
                token_for_log(provided_token.as_deref()),
                200,
                Some(e.to_string()),
            )
            .await;
            return Ok(resp);
        }
    };

    let api_key = app_state
        .providers
        .get_provider_keys(&provider.name, &app_state.config.logging.key_log_strategy)
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .next()
        .unwrap_or_default();

    if api_key.trim().is_empty()
        && !provider_uses_inline_credentials(provider.api_type, &provider.provider_config)
    {
        let resp = Json(ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some("configuration_required".into()),
            error_message: Some("no available api key".into()),
        });
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            Some(payload.model.clone()),
            Some(provider_name),
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }

    let api_type = provider.api_type.clone();
    if !api_type.supports_test_connection() {
        let resp = Json(ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some("unsupported_provider".into()),
            error_message: Some(unsupported_provider_message(api_type)),
        });
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            Some(payload.model.clone()),
            Some(provider.name),
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }
    let resp = execute_connection_test(
        api_type,
        &base_url,
        &api_key,
        &provider.provider_config,
        &upstream_model,
    )
    .await;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &path,
        REQ_TYPE_PROVIDER_MODEL_TEST,
        Some(payload.model.clone()),
        Some(provider.name),
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;

    Ok(Json(resp))
}

pub async fn test_provider_model_draft(
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<DraftProviderModelTestPayload>,
) -> Result<Json<ProviderModelTestResponse>, GatewayError> {
    require_superadmin(&headers, &app_state).await?;

    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let path = "/providers/models/test-draft";

    let api_type = payload.api_type;
    if !api_type.supports_test_connection() {
        let resp = Json(unsupported_provider_response(api_type));
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            payload.model.clone(),
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }

    if payload.base_url.trim().is_empty() {
        let resp = Json(failure_response(
            "configuration_required",
            "base_url 不能为空",
        ));
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            payload.model.clone(),
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }

    let api_key = payload.api_key.unwrap_or_default().trim().to_string();
    if api_key.is_empty() && !provider_uses_inline_credentials(api_type, &payload.provider_config) {
        let resp = Json(failure_response(
            "configuration_required",
            "api_key 不能为空",
        ));
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            path,
            REQ_TYPE_PROVIDER_MODEL_TEST,
            payload.model.clone(),
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        return Ok(resp);
    }

    let base_url = match validate_outbound_base_url(&payload.base_url).await {
        Ok(base_url) => base_url,
        Err(err) => {
            let resp = Json(failure_response("invalid_path", err.to_string()));
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                path,
                REQ_TYPE_PROVIDER_MODEL_TEST,
                payload.model.clone(),
                None,
                token_for_log(provided_token.as_deref()),
                200,
                Some(err.to_string()),
            )
            .await;
            return Ok(resp);
        }
    };

    let resolved_model = match resolve_test_model(
        api_type,
        &base_url,
        payload.models_endpoint.as_deref(),
        &api_key,
        &payload.provider_config,
        payload.model.as_deref(),
    )
    .await
    {
        Ok(model) => model,
        Err((error_type, error_message)) => {
            let resp = Json(ProviderModelTestResponse {
                success: false,
                latency: None,
                error_type: Some(error_type),
                error_message,
            });
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                path,
                REQ_TYPE_PROVIDER_MODEL_TEST,
                payload.model.clone(),
                None,
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            return Ok(resp);
        }
    };

    let resp = execute_connection_test(
        api_type,
        &base_url,
        &api_key,
        &payload.provider_config,
        &resolved_model,
    )
    .await;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        path,
        REQ_TYPE_PROVIDER_MODEL_TEST,
        Some(resolved_model),
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;

    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::{DraftProviderModelTestPayload, map_model_discovery_error};
    use crate::config::settings::ProviderConfig;
    use crate::error::GatewayError;

    #[test]
    fn discovery_error_maps_authentication_failures() {
        let (error_type, error_message) =
            map_model_discovery_error(GatewayError::Unauthorized("bad key".into()));
        assert_eq!(error_type, "authentication_failed");
        assert_eq!(error_message.as_deref(), Some("bad key"));
    }

    #[test]
    fn discovery_error_maps_invalid_path_failures() {
        let (error_type, _) = map_model_discovery_error(GatewayError::Config(
            "上游未找到模型列表接口（404），请检查 models_endpoint".into(),
        ));
        assert_eq!(error_type, "invalid_path");
    }

    #[test]
    fn discovery_error_maps_configuration_failures() {
        let (error_type, _) = map_model_discovery_error(GatewayError::Config(
            "models_endpoint 不能为空字符串".into(),
        ));
        assert_eq!(error_type, "configuration_required");
    }

    #[test]
    fn draft_payload_provider_config_accepts_missing_and_null() {
        let missing: DraftProviderModelTestPayload = serde_json::from_value(serde_json::json!({
            "api_type": "openai",
            "base_url": "https://api.openai.com/v1",
            "api_key": "sk-test"
        }))
        .unwrap();
        assert_eq!(missing.provider_config, ProviderConfig::default());

        let explicit_null: DraftProviderModelTestPayload =
            serde_json::from_value(serde_json::json!({
                "api_type": "openai",
                "base_url": "https://api.openai.com/v1",
                "api_key": "sk-test",
                "provider_config": null
            }))
            .unwrap();
        assert_eq!(explicit_null.provider_config, ProviderConfig::default());
    }
}
