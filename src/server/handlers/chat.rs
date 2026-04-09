use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::chat_request::GatewayChatCompletionRequest;
use crate::server::request_lab::{build_request_payload_snapshot, execute_logged_chat_request};
use crate::server::streaming::stream_chat_completions;

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
        let start_time = Utc::now();
        let requested_model = request.model.clone();
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
                    Some(requested_model),
                    None,
                    None,
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        };

        let snapshot = build_request_payload_snapshot(&request, top_k)?;
        let executed = match execute_logged_chat_request(
            &app_state,
            start_time,
            request,
            top_k,
            token_str,
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_ONCE,
            Some(snapshot),
        )
        .await
        {
            Ok(executed) => executed,
            Err(ge) => {
                let code = ge.status_code().as_u16();
                crate::server::request_logging::log_simple_request(
                    &app_state,
                    start_time,
                    "POST",
                    "/v1/chat/completions",
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(requested_model),
                    None,
                    client_token_log_id.as_deref(),
                    code,
                    Some(ge.to_string()),
                )
                .await;
                return Err(ge);
            }
        };

        if let Some(body) = executed.upstream_error_body {
            let v = error_payload_to_chat_completion(
                &executed.provider_name,
                &executed.effective_model,
                &body,
            );
            return Ok(Json(v).into_response());
        }

        match executed.response {
            Ok(dual) => Ok(Json(dual.raw).into_response()),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::error_payload_to_chat_completion;
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::config::settings::{
        BalanceStrategy, LoadBalancing, LoggingConfig, PricingMode, Provider, ProviderConfig,
        ProviderType, ServerConfig,
    };
    use crate::logging::{DatabaseLogger, ModelPriceUpsert};
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::extract::{Path, Query};
    use axum::http::StatusCode;
    use axum::http::{
        HeaderMap, HeaderValue,
        header::{AUTHORIZATION, CONTENT_TYPE},
    };
    use axum::response::IntoResponse;
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
        let is_openai_error_payload =
            |v: &serde_json::Value| v.get("error").is_some() && v.get("choices").is_none();
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

    fn encode_mock_aws_eventstream_frame(payload: &Value) -> Vec<u8> {
        let payload_bytes = serde_json::to_vec(payload).unwrap();
        let total_len = (16 + payload_bytes.len()) as u32;
        let mut out = Vec::with_capacity(total_len as usize);
        out.extend_from_slice(&total_len.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&payload_bytes);
        out.extend_from_slice(&0u32.to_be_bytes());
        out
    }

    async fn spawn_mock_azure_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            Path(deployment): Path<String>,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                format!("/openai/deployments/{deployment}/chat/completions"),
                query,
                &headers,
                captured_body,
            )
            .await;

            if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
                let role_chunk = json!({
                    "id": "azure-stream-1",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": deployment,
                    "choices": [{
                        "index": 0,
                        "delta": {"role": "assistant"},
                        "finish_reason": Value::Null,
                    }],
                    "usage": Value::Null,
                })
                .to_string();
                let text_chunk = json!({
                    "id": "azure-stream-1",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": deployment,
                    "choices": [{
                        "index": 0,
                        "delta": {"content": "mock azure stream ok"},
                        "finish_reason": "stop",
                    }],
                    "usage": Value::Null,
                })
                .to_string();
                let usage_chunk = json!({
                    "id": "azure-stream-1",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": deployment,
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 5,
                        "completion_tokens": 3,
                        "total_tokens": 8,
                    }
                })
                .to_string();
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    format!(
                        "data: {role_chunk}\n\ndata: {text_chunk}\n\ndata: {usage_chunk}\n\ndata: [DONE]\n\n"
                    ),
                )
                    .into_response();
            }

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
                .into_response()
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
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                format!("/v1beta/models/{model_action}"),
                query,
                &headers,
                captured_body,
            )
            .await;

            if model_action.ends_with(":streamGenerateContent") {
                let first = json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{"text": "mock gemini "}]
                        }
                    }]
                })
                .to_string();
                let second = json!({
                    "candidates": [{
                        "content": {
                            "parts": [{"text": "stream ok"}]
                        },
                        "finishReason": "STOP"
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 7,
                        "candidatesTokenCount": 4,
                        "totalTokenCount": 11
                    }
                })
                .to_string();
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    format!("data: {first}\n\ndata: {second}\n\n"),
                )
                    .into_response();
            }

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
                .into_response()
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
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                "/v2/chat".into(),
                HashMap::new(),
                &headers,
                captured_body,
            )
            .await;

            if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "event: message-start\n",
                        "data: {\"type\":\"message-start\"}\n\n",
                        "event: content-start\n",
                        "data: {\"type\":\"content-start\",\"index\":0}\n\n",
                        "event: content-delta\n",
                        "data: {\"type\":\"content-delta\",\"index\":0,\"delta\":{\"message\":{\"content\":{\"text\":\"mock cohere \"}}}}\n\n",
                        "event: content-delta\n",
                        "data: {\"type\":\"content-delta\",\"index\":0,\"delta\":{\"message\":{\"content\":{\"text\":\"stream ok\"}}}}\n\n",
                        "event: content-end\n",
                        "data: {\"type\":\"content-end\",\"index\":0}\n\n",
                        "event: message-end\n",
                        "data: {\"type\":\"message-end\",\"delta\":{\"finish_reason\":\"COMPLETE\",\"usage\":{\"tokens\":{\"input_tokens\":9,\"output_tokens\":6}}}}\n\n"
                    ),
                )
                    .into_response();
            }

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
                .into_response()
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
        ) -> axum::response::Response {
            let is_stream = headers
                .get("accept")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("application/vnd.amazon.eventstream"));
            let suffix = if is_stream {
                "converse-stream"
            } else {
                "converse"
            };
            capture_request(
                captured,
                format!("/model/{model}/{suffix}"),
                HashMap::new(),
                &headers,
                body,
            )
            .await;

            if is_stream {
                let frames = [
                    json!({"messageStart": {"role": "assistant"}}),
                    json!({"contentBlockDelta": {"contentBlockIndex": 0, "delta": {"text": "mock aws "}}}),
                    json!({"contentBlockDelta": {"contentBlockIndex": 0, "delta": {"text": "claude stream ok"}}}),
                    json!({"messageStop": {"stopReason": "end_turn"}}),
                    json!({"metadata": {"usage": {"inputTokens": 8, "outputTokens": 5, "totalTokens": 13}, "metrics": {"latencyMs": 123}}}),
                ]
                .into_iter()
                .flat_map(|payload| encode_mock_aws_eventstream_frame(&payload))
                .collect::<Vec<u8>>();

                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "application/vnd.amazon.eventstream")],
                    axum::body::Body::from(frames),
                )
                    .into_response();
            }

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
                .into_response()
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/model/{model}/converse", post(handler))
            .route("/model/{model}/converse-stream", post(handler))
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
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                format!(
                    "/v1/projects/{project}/locations/{location}/publishers/google/models/{model_action}"
                ),
                HashMap::new(),
                &headers,
                captured_body,
            )
            .await;

            if model_action.ends_with(":streamGenerateContent") {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "application/json")],
                    concat!(
                        "{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"mock vertex \"}]}}]}\n",
                        "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"stream ok\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":6,\"totalTokenCount\":16}}\n"
                    ),
                )
                    .into_response();
            }

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
                .into_response()
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

    async fn spawn_mock_openai_compat_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                "/v1/chat/completions".into(),
                HashMap::new(),
                &headers,
                captured_body,
            )
            .await;
            if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
                let model = body
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("mock-model");
                let first_chunk = json!({
                    "id": "openai-compat-stream-1",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": {"role": "assistant", "content": "mock openai compat "},
                        "finish_reason": null
                    }]
                })
                .to_string();
                let second_chunk = json!({
                    "id": "openai-compat-stream-1",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": {"content": "stream ok"},
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 6,
                        "completion_tokens": 4,
                        "total_tokens": 10
                    }
                })
                .to_string();

                (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    format!("data: {first_chunk}\n\ndata: {second_chunk}\n\ndata: [DONE]\n\n"),
                )
                    .into_response()
            } else {
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": "openai-compat-mock-1",
                        "object": "chat.completion",
                        "created": 1,
                        "model": "mock-model",
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": "mock openai compat ok"},
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 6,
                            "completion_tokens": 4,
                            "total_tokens": 10
                        }
                    })),
                )
                    .into_response()
            }
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1/chat/completions", post(handler))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/v1"), captured)
    }

    async fn spawn_mock_baidu_ernie_server() -> (String, SharedCapturedRequests) {
        async fn token_handler(
            State(captured): State<SharedCapturedRequests>,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
        ) -> axum::response::Response {
            capture_request(
                captured,
                "/oauth/2.0/token".into(),
                query,
                &headers,
                json!(null),
            )
            .await;

            (
                StatusCode::OK,
                Json(json!({
                    "access_token": "mock-baidu-access-token",
                    "expires_in": 2592000,
                })),
            )
                .into_response()
        }

        async fn chat_handler(
            State(captured): State<SharedCapturedRequests>,
            Path(model): Path<String>,
            Query(query): Query<HashMap<String, String>>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                format!("/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/{model}"),
                query,
                &headers,
                captured_body,
            )
            .await;

            if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "data: {\"id\":\"as-baidu-stream-1\",\"sentence_id\":0,\"is_end\":false,\"is_truncated\":false,\"result\":\"mock baidu \"}\n\n",
                        "data: {\"id\":\"as-baidu-stream-1\",\"sentence_id\":1,\"is_end\":true,\"is_truncated\":false,\"result\":\"stream ok\",\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":5,\"total_tokens\":13}}\n\n"
                    ),
                )
                    .into_response();
            }

            (
                StatusCode::OK,
                Json(json!({
                    "id": "as-baidu-mock-1",
                    "result": "mock baidu ok",
                    "is_truncated": false,
                    "usage": {
                        "prompt_tokens": 8,
                        "completion_tokens": 5,
                        "total_tokens": 13
                    }
                })),
            )
                .into_response()
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/oauth/2.0/token", post(token_handler))
            .route(
                "/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/{model}",
                post(chat_handler),
            )
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }

    async fn spawn_mock_xf_spark_server() -> (String, SharedCapturedRequests) {
        async fn handler(
            State(captured): State<SharedCapturedRequests>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> axum::response::Response {
            let captured_body = body.clone();
            capture_request(
                captured,
                "/v1/chat/completions".into(),
                HashMap::new(),
                &headers,
                captured_body,
            )
            .await;

            if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
                return (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "data: {\"code\":0,\"message\":\"Success\",\"sid\":\"spark-sid-1\",\"id\":\"spark-id-1\",\"created\":1719546385,\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"mock spark \"},\"index\":0}]}\n\n",
                        "data: {\"code\":0,\"message\":\"Success\",\"sid\":\"spark-sid-1\",\"id\":\"spark-id-1\",\"created\":1719546386,\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"stream ok\"},\"index\":0}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":4,\"total_tokens\":11}}\n\n",
                        "data: [DONE]\n\n"
                    ),
                )
                    .into_response();
            }

            (
                StatusCode::OK,
                Json(json!({
                    "code": 0,
                    "message": "Success",
                    "sid": "spark-sid-1",
                    "id": "spark-id-1",
                    "created": 1719546385,
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "mock spark ok"}
                    }],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 4,
                        "total_tokens": 11
                    }
                })),
            )
                .into_response()
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/v1/chat/completions", post(handler))
            .with_state(captured.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/v1"), captured)
    }

    async fn test_app_state_with_provider_options(
        provider_name: &str,
        provider_type: ProviderType,
        base_url: &str,
        provider_config: ProviderConfig,
        upstream_model: &str,
        seed_price: bool,
        pricing_mode: PricingMode,
    ) -> (tempfile::TempDir, Arc<AppState>, String) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );
        let mut settings = test_settings(db_path.to_string_lossy().to_string());
        settings.server.pricing_mode = pricing_mode;

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
        if seed_price {
            logger
                .upsert_model_price(ModelPriceUpsert::manual(
                    provider_name,
                    upstream_model,
                    1.0,
                    1.0,
                    Some("USD".into()),
                    None,
                ))
                .await
                .unwrap();
        }

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
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        (dir, app_state, token.token)
    }

    async fn test_app_state_with_provider(
        provider_name: &str,
        provider_type: ProviderType,
        base_url: &str,
        provider_config: ProviderConfig,
        upstream_model: &str,
    ) -> (tempfile::TempDir, Arc<AppState>, String) {
        test_app_state_with_provider_options(
            provider_name,
            provider_type,
            base_url,
            provider_config,
            upstream_model,
            true,
            PricingMode::Strict,
        )
        .await
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

    async fn invoke_chat_and_collect_text(
        app_state: Arc<AppState>,
        client_token: &str,
        model: &str,
        stream: bool,
    ) -> Result<(HeaderMap, String), crate::error::GatewayError> {
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

        let response_headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Ok((response_headers, String::from_utf8(bytes.to_vec()).unwrap()))
    }

    fn stream_data_lines(body: &str) -> Vec<&str> {
        body.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect()
    }

    fn parse_stream_json_chunks(body: &str) -> Vec<Value> {
        stream_data_lines(body)
            .into_iter()
            .filter(|line| *line != "[DONE]")
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect()
    }

    fn collect_stream_content(body: &str) -> String {
        parse_stream_json_chunks(body)
            .into_iter()
            .filter_map(|chunk| {
                chunk
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|choices| choices.first())
                    .and_then(|choice| choice.get("delta"))
                    .and_then(|delta| delta.get("content"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>()
            .join("")
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
    async fn mock_runtime_azure_openai_stream() {
        let (base_url, captured) = spawn_mock_azure_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "azure-stream-mock",
            ProviderType::AzureOpenAI,
            &base_url,
            ProviderConfig {
                azure_deployment: Some("gpt-4o-prod".into()),
                azure_api_version: Some("2024-06-01".into()),
                ..ProviderConfig::default()
            },
            "ignored-model-name",
        )
        .await;

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "azure-stream-mock/ignored-model-name",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock azure stream ok");

        let data_lines = stream_data_lines(&body);
        assert_eq!(data_lines.last().copied(), Some("[DONE]"));
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(
            chunks[1]["choices"][0]["delta"]["content"],
            json!("mock azure stream ok")
        );
        assert_eq!(chunks[2]["choices"][0]["finish_reason"], json!("stop"));
        assert_eq!(chunks[3]["choices"], json!([]));
        assert_eq!(chunks[3]["usage"]["total_tokens"], json!(8));

        let calls = captured.lock().await;
        let call = calls.first().expect("azure stream mock call");
        assert_eq!(
            call.path,
            "/openai/deployments/gpt-4o-prod/chat/completions"
        );
        assert_eq!(
            call.query.get("api-version"),
            Some(&"2024-06-01".to_string())
        );
        assert_eq!(
            call.headers.get("api-key"),
            Some(&"mock-upstream-key".to_string())
        );
        assert_eq!(call.body["stream"], json!(true));
        assert_eq!(call.body["stream_options"]["include_usage"], json!(true));
        assert!(call.body.get("model").is_none());
    }

    #[tokio::test]
    async fn mock_runtime_google_gemini_stream() {
        let (base_url, captured) = spawn_mock_gemini_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "gemini-stream-mock",
            ProviderType::GoogleGemini,
            &base_url,
            ProviderConfig {
                google_api_version: Some("v1beta".into()),
                ..ProviderConfig::default()
            },
            "gemini-2.0-flash",
        )
        .await;

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "gemini-stream-mock/gemini-2.0-flash",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock gemini stream ok");

        let data_lines = stream_data_lines(&body);
        assert_eq!(data_lines.last().copied(), Some("[DONE]"));
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks.last().unwrap()["usage"]["total_tokens"], json!(11));

        let calls = captured.lock().await;
        let call = calls.first().expect("gemini stream mock call");
        assert_eq!(
            call.path,
            "/v1beta/models/gemini-2.0-flash:streamGenerateContent"
        );
        assert_eq!(call.query.get("alt"), Some(&"sse".to_string()));
        assert_eq!(
            call.query.get("key"),
            Some(&"mock-upstream-key".to_string())
        );
        assert_eq!(call.body["contents"][0]["parts"][0]["text"], json!("hello"));
    }

    #[tokio::test]
    async fn mock_runtime_cohere_stream() {
        let (base_url, captured) = spawn_mock_cohere_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "cohere-stream-mock",
            ProviderType::Cohere,
            &base_url,
            ProviderConfig::default(),
            "command-r-plus",
        )
        .await;

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "cohere-stream-mock/command-r-plus",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock cohere stream ok");

        let data_lines = stream_data_lines(&body);
        assert_eq!(data_lines.last().copied(), Some("[DONE]"));
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks.last().unwrap()["usage"]["total_tokens"], json!(15));

        let calls = captured.lock().await;
        let call = calls.first().expect("cohere stream mock call");
        assert_eq!(call.path, "/v2/chat");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["stream"], json!(true));
    }

    #[tokio::test]
    async fn mock_runtime_aws_claude_stream() {
        let (base_url, captured) = spawn_mock_aws_claude_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "aws-stream-mock",
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

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "aws-stream-mock/anthropic.claude-3-5-sonnet-20241022-v2:0",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock aws claude stream ok");

        let data_lines = stream_data_lines(&body);
        assert_eq!(data_lines.last().copied(), Some("[DONE]"));
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks.last().unwrap()["usage"]["total_tokens"], json!(13));

        let calls = captured.lock().await;
        let call = calls.first().expect("aws stream mock call");
        assert_eq!(
            call.path,
            "/model/anthropic.claude-3-5-sonnet-20241022-v2:0/converse-stream"
        );
        assert!(call.headers.contains_key("authorization"));
        assert_eq!(
            call.headers.get("accept"),
            Some(&"application/vnd.amazon.eventstream".to_string())
        );
    }

    #[tokio::test]
    async fn mock_runtime_vertex_ai_stream() {
        let (base_url, captured) = spawn_mock_vertex_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "vertex-stream-mock",
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

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "vertex-stream-mock/gemini-2.0-flash-001",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock vertex stream ok");

        let data_lines = stream_data_lines(&body);
        assert_eq!(data_lines.last().copied(), Some("[DONE]"));
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks.last().unwrap()["usage"]["total_tokens"], json!(16));

        let calls = captured.lock().await;
        let call = calls.first().expect("vertex stream mock call");
        assert_eq!(
            call.path,
            "/v1/projects/demo-project/locations/us-central1/publishers/google/models/gemini-2.0-flash-001:streamGenerateContent"
        );
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer ya29.vertex-test".to_string())
        );
    }

    #[tokio::test]
    async fn mock_runtime_360_zhinao_chat() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "zhinao-mock",
            ProviderType::ThreeSixtyZhinao,
            &base_url,
            ProviderConfig::default(),
            "360gpt-pro",
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state, &token, "zhinao-mock/360gpt-pro", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock openai compat ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("360 zhinao mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["model"], json!("360gpt-pro"));
    }

    #[tokio::test]
    async fn mock_runtime_stepfun_chat() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "stepfun-mock",
            ProviderType::StepFun,
            &base_url,
            ProviderConfig::default(),
            "step-1-8k",
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state, &token, "stepfun-mock/step-1-8k", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock openai compat ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("stepfun mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["model"], json!("step-1-8k"));
    }

    #[tokio::test]
    async fn mock_runtime_openai_compatible_providers_stream() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let cases = [
            (
                "minimax-stream-mock",
                ProviderType::MiniMax,
                "MiniMax-Text-01",
            ),
            (
                "hunyuan-stream-mock",
                ProviderType::TencentHunyuan,
                "hunyuan-lite",
            ),
            (
                "zhinao-stream-mock",
                ProviderType::ThreeSixtyZhinao,
                "360gpt-pro",
            ),
            ("stepfun-stream-mock", ProviderType::StepFun, "step-1-8k"),
        ];

        for (provider_name, provider_type, upstream_model) in cases {
            let (_dir, app_state, token) = test_app_state_with_provider(
                provider_name,
                provider_type,
                &base_url,
                ProviderConfig::default(),
                upstream_model,
            )
            .await;

            let (headers, body) = invoke_chat_and_collect_text(
                app_state,
                &token,
                &format!("{provider_name}/{upstream_model}"),
                true,
            )
            .await
            .unwrap();

            assert!(
                headers
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .contains("text/event-stream")
            );

            let data_lines: Vec<_> = body
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .collect();
            assert_eq!(data_lines.last().copied(), Some("[DONE]"));
            assert_eq!(data_lines.len(), 3);

            let first_chunk: Value = serde_json::from_str(data_lines[0]).unwrap();
            let second_chunk: Value = serde_json::from_str(data_lines[1]).unwrap();
            assert_eq!(
                first_chunk["choices"][0]["delta"]["role"],
                json!("assistant")
            );
            assert_eq!(
                first_chunk["choices"][0]["delta"]["content"],
                json!("mock openai compat ")
            );
            assert_eq!(
                second_chunk["choices"][0]["delta"]["content"],
                json!("stream ok")
            );
            assert_eq!(second_chunk["choices"][0]["finish_reason"], json!("stop"));
            assert_eq!(second_chunk["usage"]["total_tokens"], json!(10));
        }

        let calls = captured.lock().await;
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].body["model"], json!("MiniMax-Text-01"));
        assert_eq!(calls[1].body["model"], json!("hunyuan-lite"));
        assert_eq!(calls[2].body["model"], json!("360gpt-pro"));
        assert_eq!(calls[3].body["model"], json!("step-1-8k"));
        for call in calls.iter() {
            assert_eq!(call.path, "/v1/chat/completions");
            assert_eq!(
                call.headers.get("authorization"),
                Some(&"Bearer mock-upstream-key".to_string())
            );
            assert_eq!(call.body["stream"], json!(true));
            assert_eq!(call.body["stream_options"]["include_usage"], json!(true));
        }
    }

    #[tokio::test]
    async fn mock_runtime_baidu_ernie_v2_chat() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "ernie-v2-mock",
            ProviderType::BaiduErnieV2,
            &base_url,
            ProviderConfig::default(),
            "ernie-4.0-turbo-8k",
        )
        .await;

        let payload = invoke_chat_and_parse_json(
            app_state,
            &token,
            "ernie-v2-mock/ernie-4.0-turbo-8k",
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock openai compat ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("baidu ernie v2 mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["model"], json!("ernie-4.0-turbo-8k"));
    }

    #[tokio::test]
    async fn mock_runtime_baidu_ernie_stream() {
        let (base_url, captured) = spawn_mock_baidu_ernie_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "ernie-stream-mock",
            ProviderType::BaiduErnie,
            &base_url,
            ProviderConfig {
                baidu_access_key: Some("mock-ak".into()),
                baidu_secret_key: Some("mock-sk".into()),
                ..ProviderConfig::default()
            },
            "completions_pro",
        )
        .await;

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "ernie-stream-mock/completions_pro",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock baidu stream ok");
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(
            chunks[1]["choices"][0]["delta"]["content"],
            json!("mock baidu ")
        );
        assert_eq!(
            chunks[2]["choices"][0]["delta"]["content"],
            json!("stream ok")
        );
        assert_eq!(
            chunks
                .iter()
                .find(|chunk| chunk.get("usage").is_some())
                .unwrap()["usage"]["total_tokens"],
            json!(13)
        );
        assert_eq!(
            chunks
                .iter()
                .find(|chunk| {
                    chunk
                        .get("choices")
                        .and_then(Value::as_array)
                        .and_then(|choices| choices.first())
                        .and_then(|choice| choice.get("finish_reason"))
                        .and_then(Value::as_str)
                        == Some("stop")
                })
                .unwrap()["choices"][0]["finish_reason"],
            json!("stop")
        );
        assert_eq!(stream_data_lines(&body).last().copied(), Some("[DONE]"));

        let calls = captured.lock().await;
        assert_eq!(calls.len(), 2);
        let token_call = &calls[0];
        assert_eq!(token_call.path, "/oauth/2.0/token");
        assert_eq!(
            token_call.query.get("client_id"),
            Some(&"mock-ak".to_string())
        );
        assert_eq!(
            token_call.query.get("client_secret"),
            Some(&"mock-sk".to_string())
        );
        let chat_call = &calls[1];
        assert_eq!(
            chat_call.path,
            "/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/completions_pro"
        );
        assert_eq!(
            chat_call.query.get("access_token"),
            Some(&"mock-baidu-access-token".to_string())
        );
        assert_eq!(chat_call.body["stream"], json!(true));
        assert_eq!(chat_call.body["max_output_tokens"], json!(16));
    }

    #[tokio::test]
    async fn mock_runtime_baidu_ernie_v2_stream() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "ernie-v2-stream-mock",
            ProviderType::BaiduErnieV2,
            &base_url,
            ProviderConfig::default(),
            "ernie-4.0-turbo-8k",
        )
        .await;

        let (headers, body) = invoke_chat_and_collect_text(
            app_state,
            &token,
            "ernie-v2-stream-mock/ernie-4.0-turbo-8k",
            true,
        )
        .await
        .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(
            collect_stream_content(&body),
            "mock openai compat stream ok"
        );
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(chunks[1]["choices"][0]["finish_reason"], json!("stop"));
        assert_eq!(chunks[1]["usage"]["total_tokens"], json!(10));
        assert_eq!(stream_data_lines(&body).last().copied(), Some("[DONE]"));

        let calls = captured.lock().await;
        let call = calls.first().expect("baidu ernie v2 stream mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(call.body["model"], json!("ernie-4.0-turbo-8k"));
        assert_eq!(call.body["stream_options"]["include_usage"], json!(true));
    }

    #[tokio::test]
    async fn mock_runtime_xf_spark_chat() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "spark-mock",
            ProviderType::XfSpark,
            &base_url,
            ProviderConfig::default(),
            "generalv3.5",
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state, &token, "spark-mock/generalv3.5", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock openai compat ok")
        );

        let calls = captured.lock().await;
        let call = calls.first().expect("xf spark mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["model"], json!("generalv3.5"));
    }

    #[tokio::test]
    async fn mock_runtime_xf_spark_stream() {
        let (base_url, captured) = spawn_mock_xf_spark_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider(
            "spark-stream-mock",
            ProviderType::XfSpark,
            &base_url,
            ProviderConfig::default(),
            "generalv3.5",
        )
        .await;

        let (headers, body) =
            invoke_chat_and_collect_text(app_state, &token, "spark-stream-mock/generalv3.5", true)
                .await
                .unwrap();

        assert!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("text/event-stream")
        );
        assert_eq!(collect_stream_content(&body), "mock spark stream ok");
        let chunks = parse_stream_json_chunks(&body);
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], json!("assistant"));
        assert_eq!(
            chunks[1]["choices"][0]["delta"]["content"],
            json!("mock spark ")
        );
        assert_eq!(
            chunks[2]["choices"][0]["delta"]["content"],
            json!("stream ok")
        );
        assert_eq!(
            chunks
                .iter()
                .find(|chunk| chunk.get("usage").is_some())
                .unwrap()["usage"]["total_tokens"],
            json!(11)
        );
        assert_eq!(
            chunks
                .iter()
                .find(|chunk| {
                    chunk
                        .get("choices")
                        .and_then(Value::as_array)
                        .and_then(|choices| choices.first())
                        .and_then(|choice| choice.get("finish_reason"))
                        .and_then(Value::as_str)
                        == Some("stop")
                })
                .unwrap()["choices"][0]["finish_reason"],
            json!("stop")
        );
        assert_eq!(stream_data_lines(&body).last().copied(), Some("[DONE]"));

        let calls = captured.lock().await;
        let call = calls.first().expect("xf spark stream mock call");
        assert_eq!(call.path, "/v1/chat/completions");
        assert_eq!(
            call.headers.get("authorization"),
            Some(&"Bearer mock-upstream-key".to_string())
        );
        assert_eq!(call.body["stream"], json!(true));
        assert!(call.body.get("stream_options").is_none());
    }

    #[tokio::test]
    async fn missing_price_strict_mode_rejects_non_stream_chat() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider_options(
            "missing-price-strict",
            ProviderType::OpenAI,
            &base_url,
            ProviderConfig::default(),
            "m1",
            false,
            PricingMode::Strict,
        )
        .await;

        let err =
            invoke_chat_and_parse_json(app_state.clone(), &token, "missing-price-strict/m1", false)
                .await
                .unwrap_err();
        assert!(err.to_string().contains("model price not set"));
        assert!(captured.lock().await.is_empty());

        let logs = app_state.log_store.get_request_logs(5, None).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].amount_spent, None);
        assert!(
            logs[0]
                .error_message
                .as_deref()
                .unwrap_or_default()
                .contains("model price not set")
        );
    }

    #[tokio::test]
    async fn missing_price_allow_missing_allows_non_stream_chat_without_amount() {
        let (base_url, captured) = spawn_mock_openai_compat_server().await;
        let (_dir, app_state, token) = test_app_state_with_provider_options(
            "missing-price-allow",
            ProviderType::OpenAI,
            &base_url,
            ProviderConfig::default(),
            "m1",
            false,
            PricingMode::AllowMissing,
        )
        .await;

        let payload =
            invoke_chat_and_parse_json(app_state.clone(), &token, "missing-price-allow/m1", false)
                .await
                .unwrap();
        assert_eq!(
            payload["choices"][0]["message"]["content"],
            json!("mock openai compat ok")
        );

        let calls = captured.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].body["model"], json!("m1"));
        drop(calls);

        let updated = app_state
            .token_store
            .get_token(&token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.amount_spent, 0.0);
        assert_eq!(updated.total_tokens_spent, 10);

        let logs = app_state.log_store.get_request_logs(5, None).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status_code, 200);
        assert_eq!(logs[0].amount_spent, None);
        assert_eq!(logs[0].total_tokens, Some(10));
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
            organizations: logger.clone(),
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
