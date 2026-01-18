use axum::http::HeaderMap;
use axum::{
    extract::{Json, State},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

// Reuse API key hint from shared server utilities
use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::AppState;
use crate::server::model_redirect::{
    apply_model_redirects, apply_provider_model_redirects_to_parsed_model,
    apply_provider_model_redirects_to_request,
};
use crate::server::provider_dispatch::select_provider_for_model;

mod common;
mod openai;
mod zhipu;

/// Chat Completions 流式入口：
/// - 仅接受 `stream=true` 的请求，否则直接报错
/// - 应用模型重定向后，根据模型选择具体 Provider，并校验令牌额度/过期/模型白名单
/// - 按 Provider 类型分发到对应的流式实现（OpenAI/Zhipu），并统一返回 SSE 响应
pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    if !request.stream.unwrap_or(false) {
        return Err(GatewayError::Config(
            "stream=false for streaming endpoint".into(),
        ));
    }

    let start_time = Utc::now();
    apply_model_redirects(&mut request);
    let parsed_for_prefix = crate::server::model_parser::ParsedModel::parse(&request.model);
    if let Some(p) = parsed_for_prefix.provider_name.as_deref() {
        if let Some((from, to)) =
            apply_provider_model_redirects_to_request(&app_state, p, &mut request).await?
        {
            tracing::info!(
                provider = p,
                source_model = %from,
                target_model = %to,
                "已应用 provider 维度模型重定向（前缀指定）"
            );
        }
    }
    let (selected, mut parsed_model) =
        select_provider_for_model(&app_state, &request.model).await?;
    if let Some((from, to)) = apply_provider_model_redirects_to_parsed_model(
        &app_state,
        &selected.provider.name,
        &mut parsed_model,
    )
    .await?
    {
        tracing::info!(
            provider = %selected.provider.name,
            source_model = %from,
            target_model = %to,
            "已应用 provider 维度模型重定向"
        );
        request.model = if parsed_model.provider_name.is_some() {
            format!("{}/{}", selected.provider.name, parsed_model.model_name)
        } else {
            parsed_model.model_name.clone()
        };
    }

    // Build upstream request with real model id
    let mut upstream_req = request.clone();
    upstream_req.model = parsed_model.get_upstream_model_name().to_string();

    // Extract required gateway token from Authorization header
    let client_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let token_str = match client_token.as_deref() {
        Some(tok) => tok,
        None => {
            let ge = GatewayError::Config("missing bearer token".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                None,
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

    let token_record = app_state.token_store.get_token(token_str).await?;
    let token = match token_record {
        Some(t) => t,
        None => {
            let ge = GatewayError::Config("invalid token".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                client_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

    if !token.enabled {
        if let Some(max_amount) = token.max_amount
            && let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(token_str)
                .await
            && spent >= max_amount
        {
            let ge = GatewayError::Config("token budget exceeded".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                client_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let ge = GatewayError::Config("token disabled".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if let Some(exp) = token.expires_at
        && chrono::Utc::now() > exp
    {
        return Err(GatewayError::Config("token expired".into()));
    }

    crate::server::token_model_limits::enforce_model_allowed_for_token(&token, &request.model)?;

    if let Some(max_tokens) = token.max_tokens
        && token.total_tokens_spent >= max_tokens
    {
        let ge = GatewayError::Config("token tokens exceeded".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if let Some(max_amount) = token.max_amount
        && let Ok(spent) = app_state
            .log_store
            .sum_spent_amount_by_client_token(token_str)
            .await
        && spent > max_amount
    {
        return Err(GatewayError::Config("token budget exceeded".into()));
    }

    let upstream_model_for_check = parsed_model.get_upstream_model_name().to_string();
    if let Ok(Some(false)) = app_state
        .log_store
        .get_model_enabled(&selected.provider.name, &upstream_model_for_check)
        .await
    {
        let ge = GatewayError::Config("model is disabled".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }
    let price = app_state
        .log_store
        .get_model_price(&selected.provider.name, &upstream_model_for_check)
        .await
        .map_err(GatewayError::Db)?;
    if price.is_none() {
        let ge = GatewayError::Config("model price not set".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let response = match selected.provider.api_type {
        crate::config::ProviderType::OpenAI => openai::stream_openai_chat(
            app_state.clone(),
            start_time,
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            client_token.clone(),
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Zhipu => zhipu::stream_zhipu_chat(
            app_state.clone(),
            start_time,
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            client_token.clone(),
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Anthropic => Err(GatewayError::Config(
            "Anthropic streaming not implemented".into(),
        )),
    };

    if let Some(tok) = client_token.as_deref()
        && let Some(t) = app_state.token_store.get_token(tok).await?
    {
        if let Some(max_amount) = t.max_amount
            && t.amount_spent > max_amount
        {
            let _ = app_state.token_store.set_enabled(tok, false).await;
        }
        if let Some(max_tokens) = t.max_tokens
            && t.total_tokens_spent > max_tokens
        {
            let _ = app_state.token_store.set_enabled(tok, false).await;
        }
    }

    response
}
