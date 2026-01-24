use std::{convert::Infallible, sync::Arc};

use axum::response::{IntoResponse, Response, Sse};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::error::GatewayError;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::server::AppState;
use crate::server::util::mask_key;

/// Anthropic streaming (best-effort):
/// - Anthropic upstream streaming is not implemented here yet.
/// - We call upstream in non-stream mode, then emit an OpenAI-compatible SSE stream containing
///   the full assistant message as a single delta chunk, followed by a finish chunk and `[DONE]`.
#[allow(clippy::too_many_arguments)]
pub async fn stream_anthropic_chat(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model_with_prefix: String,
    requested_model: String,
    effective_model: String,
    base_url: String,
    provider_name: String,
    api_key: String,
    client_token: Option<String>,
    mut upstream_req: ChatCompletionRequest,
    top_k: Option<u32>,
) -> Result<Response, GatewayError> {
    // Ensure we don't accidentally request upstream SSE.
    upstream_req.stream = Some(false);
    upstream_req.stream_options = None;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
    let api_key_ref = Some(mask_key(&api_key));
    let app_state_clone = app_state.clone();
    let client_token_for_task = client_token.clone();

    tokio::spawn(async move {
        let params =
            AnthropicProvider::convert_openai_to_anthropic_with_top_k(&upstream_req, top_k);

        let resp = AnthropicProvider::chat_completions(&base_url, &api_key, &params).await;

        match resp {
            Ok(ok) => {
                let openai_resp = AnthropicProvider::convert_anthropic_to_openai(&ok);
                let reasoning = AnthropicProvider::extract_reasoning_content(&ok);
                let usage: Option<Usage> = openai_resp.usage.clone();

                let content = openai_resp
                    .choices
                    .first()
                    .and_then(|c| c.message.content.clone())
                    .unwrap_or_default();

                let created = Utc::now().timestamp().max(0) as u64;
                let id = openai_resp.id.clone();

                let mut delta = serde_json::Map::new();
                delta.insert("role".to_string(), Value::String("assistant".to_string()));
                if !content.is_empty() {
                    delta.insert("content".to_string(), Value::String(content));
                }
                if let Some(r) = reasoning.as_deref()
                    && !r.is_empty()
                {
                    delta.insert(
                        "reasoning_content".to_string(),
                        Value::String(r.to_string()),
                    );
                }

                let chunk1 = json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": effective_model,
                    "choices": [{
                        "index": 0,
                        "delta": Value::Object(delta),
                        "finish_reason": Value::Null
                    }]
                });

                let mut chunk2 = json!({
                    "id": openai_resp.id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": openai_resp.model,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "stop"
                    }]
                });
                if let Some(u) = usage.clone()
                    && let Ok(v) = serde_json::to_value(u)
                {
                    chunk2["usage"] = v;
                }

                let _ = tx.send(axum::response::sse::Event::default().data(chunk1.to_string()));
                let _ = tx.send(axum::response::sse::Event::default().data(chunk2.to_string()));
                let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));

                tokio::spawn({
                    let app = app_state_clone.clone();
                    let billing_model = model_with_prefix.clone();
                    let requested_model = requested_model.clone();
                    let effective_model = effective_model.clone();
                    let provider = provider_name.clone();
                    let api_key = api_key_ref.clone();
                    let ct = client_token_for_task.clone();
                    async move {
                        super::common::log_stream_success(
                            app,
                            start_time,
                            billing_model,
                            requested_model,
                            effective_model,
                            provider,
                            api_key,
                            ct,
                            usage,
                        )
                        .await;
                    }
                });
            }
            Err(e) => {
                let msg = e.to_string();
                tokio::spawn({
                    let app = app_state_clone.clone();
                    let billing_model = model_with_prefix.clone();
                    let requested_model = requested_model.clone();
                    let effective_model = effective_model.clone();
                    let provider = provider_name.clone();
                    let api_key = api_key_ref.clone();
                    let ct = client_token_for_task.clone();
                    let msg = msg.clone();
                    async move {
                        super::common::log_stream_error(
                            app,
                            start_time,
                            billing_model,
                            requested_model,
                            effective_model,
                            provider,
                            api_key,
                            ct,
                            msg,
                        )
                        .await;
                    }
                });
                let _ =
                    tx.send(axum::response::sse::Event::default().data(format!("error: {}", msg)));
                let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));
            }
        }
    });

    let out_stream = tokio_stream::StreamExt::map(
        tokio_stream::wrappers::UnboundedReceiverStream::new(rx),
        Ok::<_, Infallible>,
    );
    Ok(Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}
