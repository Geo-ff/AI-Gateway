use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::server::chat_request::GatewayChatCompletionRequest;
use crate::server::provider_dispatch::{
    call_provider_with_parsed_model, select_provider_for_model,
};
use crate::server::streaming::stream_chat_completions;
use crate::server::{
    AppState,
    model_redirect::{apply_model_redirects, apply_provider_model_redirects_to_parsed_model},
    request_logging::log_chat_request,
};

fn is_openai_error_payload(v: &serde_json::Value) -> bool {
    // OpenAI-style error payload is `{ "error": { ... } }` without `choices`.
    v.get("error").is_some() && v.get("choices").is_none()
}

fn error_payload_to_chat_completion(
    provider: &str,
    effective_model: &str,
    error: &serde_json::Value,
) -> serde_json::Value {
    let created = Utc::now().timestamp().max(0) as u64;
    let id = format!("chatcmpl-error-{}", Utc::now().timestamp_millis());
    let pretty = serde_json::to_string_pretty(error).unwrap_or_else(|_| error.to_string());
    // Make the error visible in chat UIs that otherwise would show an empty assistant message.
    // Frontends that support HTML-in-Markdown can render the title in red.
    let content = format!(
        "<span style=\"color:#ef4444\">({}) provider error</span>\n\n```json\n{}\n```",
        provider, pretty
    );
    serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": effective_model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }]
    })
}

pub async fn chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(gateway_req): Json<GatewayChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    let top_k = gateway_req.top_k;
    let request = gateway_req.request;
    if request.stream.unwrap_or(false) {
        let response = stream_chat_completions(
            State(app_state),
            headers,
            Json(GatewayChatCompletionRequest { request, top_k }),
        )
        .await?;
        Ok(response.into_response())
    } else {
        let mut request = request;
        let start_time = Utc::now();
        let requested_model = request.model.clone();
        apply_model_redirects(&mut request);
        // If request pins a provider, redirected source models should be rejected (not rewritten).
        let parsed_for_prefix = crate::server::model_parser::ParsedModel::parse(&request.model);
        if let Some(p) = parsed_for_prefix.provider_name.as_deref() {
            let mut parsed = parsed_for_prefix.clone();
            if let Some((from, to)) =
                apply_provider_model_redirects_to_parsed_model(&app_state, p, &mut parsed).await?
            {
                return Err(GatewayError::Config(format!(
                    "model '{}' is redirected; use '{}' instead",
                    from, to
                )));
            }
        }

        let client_token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.to_string());
        let client_token_log_id = client_token
            .as_deref()
            .map(crate::admin::client_token_id_for_token);
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
                    client_token_log_id.as_deref(),
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        };

        if let Some(user_id) = token.user_id.as_deref() {
            let user = app_state.user_store.get_user(user_id).await?;
            let balance = user.as_ref().map(|u| u.balance).unwrap_or(0.0);
            if balance <= 0.0 {
                let _ = app_state
                    .token_store
                    .set_enabled_for_user(user_id, false)
                    .await;
                let ge =
                    GatewayError::Config("余额不足：密钥已失效；充值/订阅后需手动启用密钥".into());
                let code = ge.status_code().as_u16();
                crate::server::request_logging::log_simple_request(
                    &app_state,
                    start_time,
                    "POST",
                    "/v1/chat/completions",
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
                    client_token_log_id.as_deref(),
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        }

        if !token.enabled {
            if let Some(max_amount) = token.max_amount
                && let Ok(spent) = app_state
                    .log_store
                    .sum_spent_amount_by_client_token(&token.id)
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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
                    client_token_log_id.as_deref(),
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(request.model.clone()),
                None,
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }

        if let Some(exp) = token.expires_at
            && Utc::now() > exp
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(request.model.clone()),
                None,
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }

        if let Some(max_amount) = token.max_amount
            && let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&token.id)
                .await
            && spent > max_amount
        {
            return Err(GatewayError::Config("token budget exceeded".into()));
        }

        let (selected, mut parsed_model) =
            select_provider_for_model(&app_state, &request.model).await?;

        // 若该模型在 provider redirects 中作为 source，则不允许第三方直接调用（避免 source/target 重复可用）
        let mut parsed_for_redirect_check = parsed_model.clone();
        if let Some((from, to)) = apply_provider_model_redirects_to_parsed_model(
            &app_state,
            &selected.provider.name,
            &mut parsed_for_redirect_check,
        )
        .await?
        {
            let ge = GatewayError::Config(format!(
                "model '{}' is redirected; use '{}' instead",
                from, to
            ));
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(from),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }

        let mut redirected_from_for_price: Option<String> = None;
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
            redirected_from_for_price = Some(from.clone());
            request.model = if parsed_model.provider_name.is_some() {
                format!("{}/{}", selected.provider.name, parsed_model.model_name)
            } else {
                parsed_model.model_name.clone()
            };
        }
        let upstream_model = parsed_model.get_upstream_model_name().to_string();
        if let Ok(Some(false)) = app_state
            .log_store
            .get_model_enabled(&selected.provider.name, &upstream_model)
            .await
        {
            let ge = GatewayError::Config("model is disabled".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(upstream_model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let mut price = app_state
            .log_store
            .get_model_price(&selected.provider.name, &upstream_model)
            .await
            .map_err(GatewayError::Db)?;
        let mut effective_model_for_price = upstream_model.clone();
        if price.is_none() {
            if let Some(fallback) = redirected_from_for_price.as_deref() {
                if let Ok(p) = app_state
                    .log_store
                    .get_model_price(&selected.provider.name, fallback)
                    .await
                {
                    if p.is_some() {
                        price = p;
                        effective_model_for_price = fallback.to_string();
                    }
                }
            }
        }
        // 若客户端直接使用了重定向后的 target 模型，尝试回溯找到一个 source 模型价格（best-effort）
        if price.is_none() {
            let pairs = app_state
                .providers
                .list_model_redirects(&selected.provider.name)
                .await
                .map_err(GatewayError::Db)?;
            if !pairs.is_empty() {
                use std::collections::{HashMap, HashSet};
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

                for (source, _) in map.iter() {
                    let resolved = resolve_redirect_chain(&map, source, 16);
                    if resolved != upstream_model {
                        continue;
                    }
                    let p = app_state
                        .log_store
                        .get_model_price(&selected.provider.name, source)
                        .await
                        .map_err(GatewayError::Db)?;
                    if p.is_some() {
                        price = p;
                        effective_model_for_price = source.to_string();
                        break;
                    }
                }
            }
        }
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
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let response =
            call_provider_with_parsed_model(&selected, &request, &parsed_model, top_k).await;

        // 若上游返回 OpenAI 风格错误对象（有 error 无 choices），则对前端应当返回非 2xx，
        // 否则一些对话界面会误判为成功响应并显示空 assistant 内容。
        let upstream_error_body = response
            .as_ref()
            .ok()
            .filter(|dual| is_openai_error_payload(&dual.raw))
            .map(|dual| dual.raw.clone());

        // 日志/计费：请求日志中存 token id（不落明文），但 client_tokens 用量增量更新仍需要原始 token 值
        let token_for_log = client_token.as_deref();
        let billing_model = effective_model_for_price;
        let effective_model = upstream_model;
        if let Some(body) = upstream_error_body.as_ref() {
            let response_for_log: Result<_, GatewayError> = Err(GatewayError::Config(format!(
                "upstream returned error payload: {}",
                body
            )));
            log_chat_request(
                &app_state,
                start_time,
                &billing_model,
                &requested_model,
                &effective_model,
                &selected.provider.name,
                &selected.api_key,
                token_for_log,
                &response_for_log,
            )
            .await;
        } else {
            log_chat_request(
                &app_state,
                start_time,
                &billing_model,
                &requested_model,
                &effective_model,
                &selected.provider.name,
                &selected.api_key,
                token_for_log,
                &response,
            )
            .await;
        }

        // Auto-disable token when exceeding budget (post-check)
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

        // 将原始 JSON 透传给前端，以保留 reasoning_content 等扩展字段；
        // 但若是上游错误对象（误返回 200 且无 choices），则构造一个“带错误文本的 assistant 消息”，避免对话界面出现空回复。
        if let Some(body) = upstream_error_body {
            let v =
                error_payload_to_chat_completion(&selected.provider.name, &effective_model, &body);
            return Ok(Json(v).into_response());
        }
        match response {
            Ok(dual) => Ok(Json(dual.raw).into_response()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{error_payload_to_chat_completion, is_openai_error_payload};
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::config::settings::{
        BalanceStrategy, LoadBalancing, LoggingConfig, Provider, ProviderConfig, ProviderType,
        ServerConfig,
    };
    use crate::logging::DatabaseLogger;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::extract::{Path, Query};
    use axum::http::StatusCode;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[test]
    fn openai_error_payload_detection() {
        let v = serde_json::json!({
            "error": {
                "message": "openai_error",
                "type": "bad_response_status_code",
                "param": "",
                "code": "bad_response_status_code"
            }
        });
        assert!(is_openai_error_payload(&v));

        let ok = serde_json::json!({
            "id": "chatcmpl_x",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }]
        });
        assert!(!is_openai_error_payload(&ok));
    }

    #[test]
    fn openai_error_payload_is_rendered_as_assistant_message() {
        let err = serde_json::json!({
            "error": {
                "message": "openai_error",
                "type": "bad_response_status_code",
                "param": "",
                "code": "bad_response_status_code"
            }
        });
        let v = error_payload_to_chat_completion("fox", "m1", &err);
        assert!(v.get("choices").is_some());
        let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
        assert!(content.contains("provider error"));
        assert!(content.contains("```json"));
        assert!(content.contains("openai_error"));
    }

    fn test_settings(db_path: String) -> crate::config::Settings {
        crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path,
                ..Default::default()
            },
        }
    }

    #[derive(Debug, Clone)]
    struct CapturedUpstreamRequest {
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
        body: Value,
    }

    type SharedCapturedRequests = Arc<Mutex<Vec<CapturedUpstreamRequest>>>;

    async fn capture_request(
        captured: SharedCapturedRequests,
        path: String,
        query: HashMap<String, String>,
        headers: &HeaderMap,
        body: Value,
    ) {
        let mut normalized_headers = HashMap::new();
        for (name, value) in headers {
            if let Ok(value) = value.to_str() {
                normalized_headers.insert(name.as_str().to_string(), value.to_string());
            }
        }
        captured.lock().await.push(CapturedUpstreamRequest {
            path,
            query,
            headers: normalized_headers,
            body,
        });
    }

    async fn spawn_mock_azure_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            Path(deployment): Path<String>,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            capture_request(
                captured,
                format!("/openai/deployments/{deployment}/chat/completions"),
                query,
                &headers,
                body,
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "id": "azure-mock-1",
                    "object": "chat.completion",
                    "created": 1,
                    "model": deployment,
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "mock azure ok"},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 5,
                        "completion_tokens": 3,
                        "total_tokens": 8
                    }
                })),
            )
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/openai/deployments/{deployment}/chat/completions",
                post(handler),
            )
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn spawn_mock_gemini_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            Path(model_action): Path<String>,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            capture_request(
                captured,
                format!("/v1beta/models/{model_action}"),
                query,
                &headers,
                body,
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "responseId": "gemini-mock-1",
                    "candidates": [{
                        "finishReason": "STOP",
                        "content": {
                            "role": "model",
                            "parts": [{"text": "mock gemini ok"}]
                        }
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 7,
                        "candidatesTokenCount": 4,
                        "totalTokenCount": 11
                    }
                })),
            )
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1beta/models/{model_action}", post(handler))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn spawn_mock_cohere_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            capture_request(captured, "/v2/chat".into(), HashMap::new(), &headers, body).await;
            (
                StatusCode::OK,
                Json(json!({
                    "id": "cohere-mock-1",
                    "finish_reason": "COMPLETE",
                    "message": {
                        "role": "assistant",
                        "content": [{"type": "text", "text": "mock cohere ok"}]
                    },
                    "usage": {
                        "tokens": {
                            "input_tokens": 9,
                            "output_tokens": 6
                        }
                    }
                })),
            )
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v2/chat", post(handler))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn spawn_mock_aws_claude_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            Path(model): Path<String>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            capture_request(
                captured,
                format!("/model/{model}/converse"),
                HashMap::new(),
                &headers,
                body,
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "output": {
                        "message": {
                            "role": "assistant",
                            "content": [{"text": "mock aws claude ok"}]
                        }
                    },
                    "stopReason": "end_turn",
                    "usage": {
                        "inputTokens": 8,
                        "outputTokens": 5,
                        "totalTokens": 13
                    }
                })),
            )
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/model/{model}/converse", post(handler))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn spawn_mock_vertex_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            Path((project, location, model_action)): Path<(String, String, String)>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            capture_request(
                captured,
                format!(
                    "/v1/projects/{project}/locations/{location}/publishers/google/models/{model_action}"
                ),
                HashMap::new(),
                &headers,
                body,
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "responseId": "vertex-mock-1",
                    "candidates": [{
                        "finishReason": "STOP",
                        "content": {
                            "role": "model",
                            "parts": [{"text": "mock vertex ok"}]
                        }
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 10,
                        "candidatesTokenCount": 6,
                        "totalTokenCount": 16
                    }
                })),
            )
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/projects/{project}/locations/{location}/publishers/google/models/{model_action}",
                post(handler),
            )
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn test_app_state_with_provider(
        provider_name: &str,
        provider_type: ProviderType,
        base_url: &str,
        provider_config: ProviderConfig,
        upstream_model: &str,
    ) -> (tempfile::TempDir, Arc<AppState>, String) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );
        let settings = test_settings(db_path.to_string_lossy().to_string());

        logger
            .insert_provider(&Provider {
                name: provider_name.into(),
                display_name: None,
                collection: crate::config::settings::DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: provider_type,
                api_type_raw: None,
                base_url: base_url.into(),
                api_keys: Vec::new(),
                models_endpoint: None,
                provider_config,
                enabled: true,
                created_at: None,
                updated_at: None,
            })
            .await
            .unwrap();
        logger
            .add_provider_key(
                provider_name,
                "mock-upstream-key",
                &settings.logging.key_log_strategy,
            )
            .await
            .unwrap();
        logger
            .upsert_model_price(provider_name, upstream_model, 1.0, 1.0, Some("USD"), None)
            .await
            .unwrap();

        let token = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: None,
                name: Some(format!("{provider_name}-token")),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        (dir, app_state, token.token)
    }

    async fn invoke_chat_and_parse_json(
        app_state: Arc<AppState>,
        client_token: &str,
        model: &str,
        stream: bool,
    ) -> Result<Value, crate::error::GatewayError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {client_token}")).unwrap(),
        );
        let request: crate::providers::openai::ChatCompletionRequest =
            serde_json::from_value(json!({
                "model": model,
                "messages": [{"role":"system","content":"You are a test assistant"},{"role":"user","content":"hello"}],
                "stream": stream,
                "max_tokens": 16,
                "temperature": 0
            }))
            .unwrap();

        let response = super::chat_completions(
            State(app_state),
            headers,
            Json(super::GatewayChatCompletionRequest {
                request,
                top_k: None,
            }),
        )
        .await?;

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Ok(serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn mock_runtime_azure_openai_chat() {
        let (base_url, captured) = spawn_mock_azure_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "azure-mock",
            ProviderType::AzureOpenAI,
            &base_url,
            ProviderConfig {
                azure_deployment: Some("gpt-4o-deploy".into()),
                azure_api_version: Some("2024-06-01".into()),
                google_api_version: None,
                ..ProviderConfig::default()
            },
            "gpt-4o",
        )
        .await;

        let payload = invoke_chat_and_parse_json(app_state, &token, "azure-mock/gpt-4o", false)
            .await
            .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock azure ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("azure mock call");
        assert_eq!(
            call.path,
            "/openai/deployments/gpt-4o-deploy/chat/completions"
        );
        assert_eq!(
            call.query.get("api-version"),
            Some(&"2024-06-01".to_string())
        );
        assert_eq!(
            call.headers.get("api-key"),
            Some(&"mock-upstream-key".to_string())
        );
        assert!(call.body.get("model").is_none());
    }

    #[tokio::test]
    async fn mock_runtime_google_gemini_chat() {
        let (base_url, captured) = spawn_mock_gemini_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "gemini-mock",
            ProviderType::GoogleGemini,
            &base_url,
            ProviderConfig {
                azure_deployment: None,
                azure_api_version: None,
                google_api_version: Some("v1beta".into()),
                ..ProviderConfig::default()
            },
            "gemini-2.0-flash",
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state, &token, "gemini-mock/gemini-2.0-flash", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock gemini ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("gemini mock call");
        assert_eq!(call.path, "/v1beta/models/gemini-2.0-flash:generateContent");
        assert_eq!(
            call.query.get("key"),
            Some(&"mock-upstream-key".to_string())
        );
        assert_eq!(
            call.body["systemInstruction"]["parts"][0]["text"],
            json!("You are a test assistant")
        );
        assert_eq!(call.body["contents"][0]["role"], json!("user"));
    }

    #[tokio::test]
    async fn mock_runtime_cohere_chat() {
        let (base_url, captured) = spawn_mock_cohere_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "cohere-mock",
            ProviderType::Cohere,
            &base_url,
            ProviderConfig::default(),
            "command-r-plus",
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state, &token, "cohere-mock/command-r-plus", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock cohere ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("cohere mock call");
        assert_eq!(call.path, "/v2/chat");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["model"], json!("command-r-plus"));
        assert_eq!(call.body["preamble"], json!("You are a test assistant"));
    }

    #[tokio::test]
    async fn mock_runtime_aws_claude_chat() {
        let (base_url, captured) = spawn_mock_aws_claude_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "aws-claude-mock",
            ProviderType::AwsClaude,
            &base_url,
            ProviderConfig {
                aws_region: Some("us-west-2".into()),
                aws_access_key_id: Some("AKIA_TEST".into()),
                aws_secret_access_key: Some("secret-test".into()),
                ..ProviderConfig::default()
            },
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
        )
        .await;

        let payload = invoke_chat_and_parse_json(
            app_state,
            &token,
            "aws-claude-mock/anthropic.claude-3-5-sonnet-20241022-v2:0",
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock aws claude ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("aws claude mock call");
        assert_eq!(
            call.path,
            "/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse"
        );
        assert!(call.headers.contains_key("authorization"));
        assert!(call.headers.contains_key("x-amz-date"));
        assert!(call.headers.contains_key("x-amz-content-sha256"));
        assert_eq!(
            call.body["system"][0]["text"],
            json!("You are a test assistant")
        );
    }

    #[tokio::test]
    async fn mock_runtime_vertex_ai_chat() {
        let (base_url, captured) = spawn_mock_vertex_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "vertex-mock",
            ProviderType::VertexAI,
            &base_url,
            ProviderConfig {
                vertex_project_id: Some("demo-project".into()),
                vertex_location: Some("us-central1".into()),
                vertex_access_token: Some("ya29.vertex-test".into()),
                ..ProviderConfig::default()
            },
            "gemini-2.0-flash-001",
        )
        .await;

        let payload = invoke_chat_and_parse_json(
            app_state,
            &token,
            "vertex-mock/gemini-2.0-flash-001",
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock vertex ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("vertex mock call");
        assert_eq!(
            call.path,
            "/v1/projects/demo-project/locations/us-central1/publishers/google/models/gemini-2.0-flash-001:generateContent"
        );
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer ya29.vertex-test".to_string())
        );
        assert_eq!(call.body["contents"][0]["role"], json!("user"));
    }

    #[tokio::test]
    async fn mock_runtime_new_native_providers_reject_stream() {
        let (base_url, _captured) = spawn_mock_gemini_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "gemini-stream-mock",
            ProviderType::GoogleGemini,
            &base_url,
            ProviderConfig {
                azure_deployment: None,
                azure_api_version: None,
                google_api_version: Some("v1beta".into()),
                ..ProviderConfig::default()
            },
            "gemini-2.0-flash",
        )
        .await;

        let err = invoke_chat_and_parse_json(
            app_state,
            &token,
            "gemini-stream-mock/gemini-2.0-flash",
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("仅支持非流式真实请求"));

        let (aws_base_url, _captured) = spawn_mock_aws_claude_server().await;
        let (_dir, aws_app_state, aws_token) = test_app_state_with_provider(
            "aws-stream-mock",
            ProviderType::AwsClaude,
            &aws_base_url,
            ProviderConfig {
                aws_region: Some("us-west-2".into()),
                aws_access_key_id: Some("AKIA_TEST".into()),
                aws_secret_access_key: Some("secret-test".into()),
                ..ProviderConfig::default()
            },
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
        )
        .await;

        let aws_err = invoke_chat_and_parse_json(
            aws_app_state,
            &aws_token,
            "aws-stream-mock/anthropic.claude-3-5-sonnet-20241022-v2:0",
            true,
        )
        .await
        .unwrap_err();
        assert!(aws_err.to_string().contains("仅支持非流式真实请求"));

        let (vertex_base_url, _captured) = spawn_mock_vertex_server().await;
        let (_dir, vertex_app_state, vertex_token) = test_app_state_with_provider(
            "vertex-stream-mock",
            ProviderType::VertexAI,
            &vertex_base_url,
            ProviderConfig {
                vertex_project_id: Some("demo-project".into()),
                vertex_location: Some("us-central1".into()),
                vertex_access_token: Some("ya29.vertex-test".into()),
                ..ProviderConfig::default()
            },
            "gemini-2.0-flash-001",
        )
        .await;

        let vertex_err = invoke_chat_and_parse_json(
            vertex_app_state,
            &vertex_token,
            "vertex-stream-mock/gemini-2.0-flash-001",
            true,
        )
        .await
        .unwrap_err();
        assert!(vertex_err.to_string().contains("仅支持非流式真实请求"));
    }

    #[tokio::test]
    async fn user_balance_depleted_rejects_chat_and_disables_tokens() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = test_settings(db_path.to_string_lossy().to_string());
        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        let user = logger
            .create_user(CreateUserPayload {
                first_name: Some("U".into()),
                last_name: Some("1".into()),
                username: None,
                email: "u1@example.com".into(),
                phone_number: None,
                password: None,
                status: UserStatus::Active,
                role: UserRole::Admin,
                is_anonymous: false,
            })
            .await
            .unwrap();

        let t1 = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("t1".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let _t2 = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("t2".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", t1.token)).unwrap(),
        );

        let req: crate::providers::openai::ChatCompletionRequest =
            serde_json::from_value(serde_json::json!({
                "model": "m1",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false
            }))
            .unwrap();

        let err = super::chat_completions(
            State(app_state),
            headers,
            Json(super::GatewayChatCompletionRequest {
                request: req,
                top_k: None,
            }),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("余额不足"));

        let tokens = logger.list_tokens_by_user(&user.id).await.unwrap();
        assert!(!tokens.is_empty());
        assert!(tokens.iter().all(|t| !t.enabled));
    }
}
