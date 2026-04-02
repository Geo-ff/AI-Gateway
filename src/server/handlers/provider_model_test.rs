use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use super::auth::require_superadmin;
use crate::config::settings::ProviderType;
use crate::error::GatewayError;
use crate::logging::types::REQ_TYPE_PROVIDER_MODEL_TEST;
use crate::providers::adapters::{
    ConnectionTestRequest, adapter_for, unsupported_provider_message,
};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::ssrf::validate_outbound_base_url;
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Deserialize)]
pub struct ProviderModelTestPayload {
    pub model: String,
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

async fn send_test_request(
    provider_type: ProviderType,
    base_url: &reqwest::Url,
    api_key: &str,
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
        })
        .await
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

    if api_key.trim().is_empty() {
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
    let t0 = Instant::now();
    let mut outcome = send_test_request(
        api_type.clone(),
        &base_url,
        &api_key,
        &upstream_model,
        false,
    )
    .await;
    // Some upstream aggregators only work in stream mode for specific models. If we got a structured
    // upstream error indicating status/body parsing issues, retry with `stream=true` (SSE) and
    // treat any 2xx as success.
    if outcome.is_err()
        && adapter_for(api_type)
            .map(|adapter| adapter.supports_stream_retry())
            .unwrap_or(false)
        && let Err((_, Some(detail))) = &outcome
    {
        let lower = detail.to_lowercase();
        if lower.contains("bad_response_body") || lower.contains("bad_response_status_code") {
            outcome =
                send_test_request(api_type.clone(), &base_url, &api_key, &upstream_model, true)
                    .await;
        }
    }
    let latency = t0.elapsed().as_secs_f64();

    let resp = match outcome {
        Ok(()) => ProviderModelTestResponse {
            success: true,
            latency: Some(latency),
            error_type: None,
            error_message: None,
        },
        Err((error_type, error_message)) => ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some(error_type),
            error_message,
        },
    };

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
