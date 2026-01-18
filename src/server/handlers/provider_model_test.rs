use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::auth::require_superadmin;
use crate::config::settings::ProviderType;
use crate::error::GatewayError;
use crate::logging::types::REQ_TYPE_PROVIDER_MODEL_TEST;
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

fn classify_http_failure(
    status: reqwest::StatusCode,
    body_snippet: &str,
) -> (String, Option<String>) {
    let snippet = body_snippet.trim();
    let lower = snippet.to_lowercase();

    if status == reqwest::StatusCode::NOT_FOUND {
        // 404 can be either "invalid_path" or "model_not_found"; best-effort by message.
        if lower.contains("model") && (lower.contains("not found") || lower.contains("not_found")) {
            return ("model_not_found".into(), Some(snippet.to_string()));
        }
        return ("invalid_path".into(), Some(snippet.to_string()));
    }

    if status == reqwest::StatusCode::REQUEST_TIMEOUT {
        return ("timeout".into(), Some(snippet.to_string()));
    }

    if status == reqwest::StatusCode::PAYMENT_REQUIRED
        || lower.contains("insufficient")
        || lower.contains("balance")
    {
        return ("insufficient_balance".into(), Some(snippet.to_string()));
    }

    (
        "other".into(),
        Some(snippet.to_string()).filter(|s| !s.is_empty()),
    )
}

fn format_upstream_error_detail(
    status: reqwest::StatusCode,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<String> {
    let ct = content_type.unwrap_or("").trim();

    // keep JSON as pretty JSON if possible
    if ct.contains("application/json") {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
            let out = serde_json::json!({
                "status": status.as_u16(),
                "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
                "body": v,
            });
            return serde_json::to_string_pretty(&out).ok();
        }
    }

    let snippet = String::from_utf8_lossy(bytes);
    let snippet = snippet.trim();
    if snippet.is_empty() {
        let out = serde_json::json!({
            "status": status.as_u16(),
            "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
        });
        return serde_json::to_string_pretty(&out).ok();
    }

    let out = serde_json::json!({
        "status": status.as_u16(),
        "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
        "body_text": snippet,
    });
    serde_json::to_string_pretty(&out).ok()
}

fn classify_reqwest_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timeout".into();
    }
    if let Some(status) = err.status() {
        if status == reqwest::StatusCode::NOT_FOUND {
            return "invalid_path".into();
        }
    }
    "other".into()
}

async fn send_test_request(
    provider_type: ProviderType,
    base_url: &reqwest::Url,
    api_key: &str,
    model: &str,
    stream: bool,
) -> Result<(), (String, Option<String>)> {
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ("other".into(), Some(e.to_string())))?;

    let model = model.trim();
    if model.is_empty() {
        return Err((
            "model_not_found".into(),
            Some("model cannot be empty".into()),
        ));
    }

    match provider_type {
        ProviderType::OpenAI => {
            let url = format!(
                "{}/v1/chat/completions",
                base_url.as_str().trim_end_matches('/')
            );
            let payload = serde_json::json!({
              "model": model,
              "messages": [{"role":"user","content":"ping"}],
              "stream": stream,
              "max_tokens": 1,
              "temperature": 0
            });
            let resp = client
                .post(&url)
                .bearer_auth(api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| (classify_reqwest_error(&e), Some(e.to_string())))?;

            let status = resp.status();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ("other".into(), Some(e.to_string())))?;
            if !status.is_success() {
                let detail = format_upstream_error_detail(status, content_type.as_deref(), &bytes);
                let snippet = String::from_utf8_lossy(&bytes);
                let (ty, _) = classify_http_failure(status, &snippet);
                return Err((ty, detail));
            }
            Ok(())
        }
        ProviderType::Anthropic => {
            let url = format!("{}/v1/messages", base_url.as_str().trim_end_matches('/'));
            // Anthropic Messages API minimal payload
            let payload = serde_json::json!({
              "model": model,
              "stream": stream,
              "max_tokens": 1,
              "messages": [{"role":"user","content":[{"type":"text","text":"ping"}]}]
            });
            let resp = client
                .post(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| (classify_reqwest_error(&e), Some(e.to_string())))?;

            let status = resp.status();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ("other".into(), Some(e.to_string())))?;
            if !status.is_success() {
                let detail = format_upstream_error_detail(status, content_type.as_deref(), &bytes);
                let snippet = String::from_utf8_lossy(&bytes);
                let (ty, _) = classify_http_failure(status, &snippet);
                return Err((ty, detail));
            }
            Ok(())
        }
        ProviderType::Zhipu => {
            let url = format!(
                "{}/api/paas/v4/chat/completions",
                base_url.as_str().trim_end_matches('/')
            );
            let payload = serde_json::json!({
              "model": model,
              "messages": [{"role":"user","content":"ping"}],
              "stream": stream,
              "max_tokens": 1,
              "temperature": 0
            });
            let resp = client
                .post(&url)
                .bearer_auth(api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| (classify_reqwest_error(&e), Some(e.to_string())))?;

            let status = resp.status();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ("other".into(), Some(e.to_string())))?;
            if !status.is_success() {
                let detail = format_upstream_error_detail(status, content_type.as_deref(), &bytes);
                let snippet = String::from_utf8_lossy(&bytes);
                let (ty, _) = classify_http_failure(status, &snippet);
                return Err((ty, detail));
            }
            Ok(())
        }
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

    if api_key.trim().is_empty() {
        let resp = Json(ProviderModelTestResponse {
            success: false,
            latency: None,
            error_type: Some("other".into()),
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
        && matches!(api_type, ProviderType::OpenAI)
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
