use axum::{extract::{Json, State}, response::{IntoResponse, Response}};
use axum::http::HeaderMap;
use chrono::Utc;
use std::sync::Arc;

// Reuse API key hint from shared server utilities
pub(super) use crate::server::util::api_key_hint;
use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::model_redirect::apply_model_redirects;
use crate::server::provider_dispatch::select_provider_for_model;
use crate::server::AppState;

mod openai;
mod zhipu;
mod common;

pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    if !request.stream.unwrap_or(false) {
        return Err(GatewayError::Config("stream=false for streaming endpoint".into()));
    }

    let start_time = Utc::now();
    apply_model_redirects(&mut request);
    let (selected, parsed_model) = select_provider_for_model(&app_state, &request.model).await?;

    // Build upstream request with real model id
    let mut upstream_req = request.clone();
    upstream_req.model = parsed_model.get_upstream_model_name().to_string();

    // Extract required gateway token from Authorization header
    let client_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let is_admin_identity = client_token.as_deref().map(|tok| tok == app_state.admin_identity_token).unwrap_or(false);
    if client_token.is_none() {
        let ge = GatewayError::Config("missing bearer token".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), None, code, Some(ge.to_string())).await;
        return Err(ge);
    }
    if !is_admin_identity {
        if let Some(tok) = client_token.as_deref() {
            if let Some(t) = app_state.token_store.get_token(tok).await? {
                if !t.enabled {
                    // 更精确地返回余额不足
                    if let Some(max_amount) = t.max_amount {
                        if let Ok(spent) = app_state.log_store.sum_spent_amount_by_client_token(tok).await {
                            if spent >= max_amount {
                                let ge = GatewayError::Config("token budget exceeded".into());
                                let code = ge.status_code().as_u16();
                                crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), client_token.as_deref(), code, Some(ge.to_string())).await;
                                return Err(ge);
                            }
                        }
                    }
                    let ge = GatewayError::Config("token disabled".into());
                    let code = ge.status_code().as_u16();
                    crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), client_token.as_deref(), code, Some(ge.to_string())).await;
                    return Err(ge);
                }
                if let Some(exp) = t.expires_at { if chrono::Utc::now() > exp { return Err(GatewayError::Config("token expired".into())); } }
                if let Some(allow) = t.allowed_models.as_ref() {
                    if !allow.iter().any(|m| m == &request.model) { return Err(GatewayError::Config("model not allowed for token".into())); }
                }
                if let Some(max_tokens) = t.max_tokens {
                    if t.total_tokens_spent >= max_tokens {
                        let ge = GatewayError::Config("token tokens exceeded".into());
                        let code = ge.status_code().as_u16();
                        crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), client_token.as_deref(), code, Some(ge.to_string())).await;
                        return Err(ge);
                    }
                }
                if let Some(max_amount) = t.max_amount {
                    if let Ok(spent) = app_state.log_store.sum_spent_amount_by_client_token(tok).await {
                        if spent > max_amount { return Err(GatewayError::Config("token budget exceeded".into())); }
                    }
                }
            } else {
                let ge = GatewayError::Config("invalid token".into());
                let code = ge.status_code().as_u16();
                crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), client_token.as_deref(), code, Some(ge.to_string())).await;
                return Err(ge);
            }
        }
    }

    // 非管理员令牌：必须设置过价格
    if !is_admin_identity {
        let upstream_model_for_check = parsed_model.get_upstream_model_name().to_string();
        let price = app_state
            .log_store
            .get_model_price(&selected.provider.name, &upstream_model_for_check)
            .await
            .map_err(GatewayError::Db)?;
        if price.is_none() {
            let ge = GatewayError::Config("model price not set".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(&app_state, start_time, "POST", "/v1/chat/completions", crate::logging::types::REQ_TYPE_CHAT_STREAM, Some(upstream_req.model.clone()), Some(selected.provider.name.clone()), client_token.as_deref(), code, Some(ge.to_string())).await;
            return Err(ge);
        }
    }

    match selected.provider.api_type {
        crate::config::ProviderType::OpenAI => openai::stream_openai_chat(
            app_state,
            start_time,
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            if is_admin_identity { Some("admin_token".to_string()) } else { client_token.clone() },
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Zhipu => zhipu::stream_zhipu_chat(
            app_state,
            start_time,
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            if is_admin_identity { Some("admin_token".to_string()) } else { client_token.clone() },
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Anthropic => {
            Err(GatewayError::Config("Anthropic streaming not implemented".into()))
        }
    }
}

// api_key_hint is re-exported above from server::util
