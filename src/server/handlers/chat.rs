use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::provider_dispatch::{
    call_provider_with_parsed_model, select_provider_for_model,
};
use crate::server::streaming::stream_chat_completions;
use crate::server::{
    AppState, model_redirect::apply_model_redirects, request_logging::log_chat_request,
};

pub async fn chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    if request.stream.unwrap_or(false) {
        let response = stream_chat_completions(State(app_state), headers, Json(request)).await?;
        Ok(response.into_response())
    } else {
        let mut request = request;
        let start_time = Utc::now();
        apply_model_redirects(&mut request);

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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
                    client_token.as_deref(),
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        };

        if !token.enabled {
            if let Some(max_amount) = token.max_amount {
                if let Ok(spent) = app_state
                    .log_store
                    .sum_spent_amount_by_client_token(token_str)
                    .await
                {
                    if spent >= max_amount {
                        let ge = GatewayError::Config("token budget exceeded".into());
                        let code = ge.status_code().as_u16();
                        crate::server::request_logging::log_simple_request(
                            &app_state,
                            start_time,
                            "POST",
                            "/v1/chat/completions",
                            crate::logging::types::REQ_TYPE_CHAT_ONCE,
                            Some(request.model.clone()),
                            None,
                            client_token.as_deref(),
                            code,
                            Some(ge.to_string()),
                        )
                        .await;
                        return Err(ge);
                    }
                }
            }
            let ge = GatewayError::Config("token disabled".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(request.model.clone()),
                None,
                client_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }

        if let Some(exp) = token.expires_at {
            if Utc::now() > exp {
                return Err(GatewayError::Config("token expired".into()));
            }
        }

        if let Some(allow) = token.allowed_models.as_ref() {
            if !allow.iter().any(|m| m == &request.model) {
                return Err(GatewayError::Config("model not allowed for token".into()));
            }
        }

        if let Some(max_tokens) = token.max_tokens {
            if token.total_tokens_spent >= max_tokens {
                let ge = GatewayError::Config("token tokens exceeded".into());
                let code = ge.status_code().as_u16();
                crate::server::request_logging::log_simple_request(
                    &app_state,
                    start_time,
                    "POST",
                    "/v1/chat/completions",
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
                    client_token.as_deref(),
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        }

        if let Some(max_amount) = token.max_amount {
            if let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(token_str)
                .await
            {
                if spent > max_amount {
                    return Err(GatewayError::Config("token budget exceeded".into()));
                }
            }
        }

        let (selected, parsed_model) =
            select_provider_for_model(&app_state, &request.model).await?;
        let upstream_model = parsed_model.get_upstream_model_name().to_string();
        let price = app_state
            .log_store
            .get_model_price(&selected.provider.name, &upstream_model)
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(upstream_model.clone()),
                Some(selected.provider.name.clone()),
                client_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let response = call_provider_with_parsed_model(&selected, &request, &parsed_model).await;

        // 日志使用 typed，用于提取 usage
        let token_for_log = client_token.as_deref();
        let logged_model = parsed_model.get_upstream_model_name().to_string();
        log_chat_request(
            &app_state,
            start_time,
            &logged_model,
            &selected.provider.name,
            &selected.api_key,
            token_for_log,
            &response,
        )
        .await;

        // Auto-disable token when exceeding budget (post-check)
        if let Some(tok) = client_token.as_deref() {
            if let Some(t) = app_state.token_store.get_token(tok).await? {
                if let Some(max_amount) = t.max_amount {
                    if t.amount_spent > max_amount {
                        let _ = app_state.token_store.set_enabled(tok, false).await;
                    }
                }
                if let Some(max_tokens) = t.max_tokens {
                    if t.total_tokens_spent > max_tokens {
                        let _ = app_state.token_store.set_enabled(tok, false).await;
                    }
                }
            }
        }

        // 将原始 JSON 透传给前端，以保留 reasoning_content 等扩展字段
        match response {
            Ok(dual) => Ok(Json(dual.raw).into_response()),
            Err(e) => Err(e),
        }
    }
}
