use std::{convert::Infallible, sync::{Arc, Mutex}};

use axum::response::{IntoResponse, Response, Sse};
use chrono::{DateTime, Utc};
use reqwest_eventsource::{Event, RequestBuilderExt};

use async_openai::types::{ChatCompletionStreamOptions, CreateChatCompletionStreamResponse, CompletionTokensDetails, PromptTokensDetails};
use serde_json::Value;

use crate::error::GatewayError;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::server::AppState;

use super::api_key_hint;

pub async fn stream_openai_chat(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model_with_prefix: String,
    base_url: String,
    provider_name: String,
    api_key: String,
    mut upstream_req: ChatCompletionRequest,
) -> Result<Response, GatewayError> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    upstream_req.stream = Some(true);
    upstream_req.stream_options = Some(ChatCompletionStreamOptions { include_usage: true });

    let request_builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&upstream_req);

    let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
    let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let api_key_ref = api_key_hint(&app_state.config.logging, &api_key);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
    let usage_cell_for_task = usage_cell.clone();
    let app_state_clone = app_state.clone();
    tokio::spawn(async move {
        let mut es = match request_builder.eventsource() {
            Ok(es) => es,
            Err(e) => {
                tracing::error!("Failed to open eventsource: {}", e);
                let state_for_log = app_state_clone.clone();
                let model_for_log = model_with_prefix.clone();
                let provider_for_log = provider_name.clone();
                let api_key_for_log = api_key_ref.clone();
                let started_at = start_time;
                let msg = e.to_string();
                tokio::spawn(async move {
                    super::common::log_stream_error(
                        state_for_log,
                        started_at,
                        model_for_log,
                        provider_for_log,
                        api_key_for_log,
                        msg,
                    )
                    .await;
                });
                let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", e)));
                return;
            }
        };

        while let Some(ev) = futures_util::StreamExt::next(&mut es).await {
            match ev {
                Ok(Event::Open) => {}
                Ok(Event::Message(m)) => {
                    if m.data.trim() == "[DONE]" {
                        if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                            tokio::spawn({
                                let app = app_state_clone.clone();
                                let model = model_with_prefix.clone();
                                let provider = provider_name.clone();
                                let api_key = api_key_ref.clone();
                                async move {
                                    super::common::log_stream_success(
                                        app,
                                        start_time,
                                        model,
                                        provider,
                                        api_key,
                                        usage_snapshot,
                                    )
                                    .await;
                                }
                            });
                        }
                        let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));
                        break;
                    }

                    // Primary: try typed parse
                    let mut captured = false;
                    if let Ok(chunk) = serde_json::from_str::<CreateChatCompletionStreamResponse>(&m.data) {
                        if let Some(u) = &chunk.usage {
                            *usage_cell_for_task.lock().unwrap() = Some(u.clone());
                            captured = true;
                        }
                    }
                    // Fallback: Value parse to extract usage (tolerate vendor extensions)
                    if !captured {
                        if let Ok(v) = serde_json::from_str::<Value>(&m.data) {
                            if let Some(u) = v.get("usage") {
                                let prompt = u.get("prompt_tokens").and_then(|x| x.as_u64()).map(|x| x as u32);
                                let completion = u.get("completion_tokens").and_then(|x| x.as_u64()).map(|x| x as u32);
                                let total = u.get("total_tokens").and_then(|x| x.as_u64()).map(|x| x as u32);
                                let cached = u
                                    .get("prompt_tokens_details")
                                    .and_then(|d| d.get("cached_tokens"))
                                    .and_then(|x| x.as_u64())
                                    .map(|x| x as u32);
                                let reasoning = u
                                    .get("completion_tokens_details")
                                    .and_then(|d| d.get("reasoning_tokens"))
                                    .and_then(|x| x.as_u64())
                                    .map(|x| x as u32);

                                if prompt.is_some() || completion.is_some() || total.is_some() || cached.is_some() || reasoning.is_some() {
                                    let usage = Usage {
                                        prompt_tokens: prompt.unwrap_or(0),
                                        completion_tokens: completion.unwrap_or(0),
                                        total_tokens: total.unwrap_or(prompt.unwrap_or(0) + completion.unwrap_or(0)),
                                        prompt_tokens_details: if cached.is_some() { Some(PromptTokensDetails { cached_tokens: cached, audio_tokens: None }) } else { None },
                                        completion_tokens_details: if reasoning.is_some() { Some(CompletionTokensDetails { reasoning_tokens: reasoning, audio_tokens: None, accepted_prediction_tokens: None, rejected_prediction_tokens: None }) } else { None },
                                    };
                                    *usage_cell_for_task.lock().unwrap() = Some(usage);
                                }
                            }
                        }
                    }

                    let _ = tx.send(axum::response::sse::Event::default().data(m.data));
                }
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    let error_msg = e.to_string();
                    if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        let state_for_log = app_state_clone.clone();
                        let model_for_log = model_with_prefix.clone();
                        let provider_for_log = provider_name.clone();
                        let api_key_for_log = api_key_ref.clone();
                        let started_at = start_time;
                        let error_for_log = error_msg.clone();
                        tokio::spawn(async move {
                            super::common::log_stream_error(
                                state_for_log,
                                started_at,
                                model_for_log,
                                provider_for_log,
                                api_key_for_log,
                                error_for_log,
                            )
                            .await;
                        });
                    }
                    let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", error_msg)));
                    break;
                }
            }
        }

        // Safety net: log if stream closed without [DONE]
        if !logged_flag.load(std::sync::atomic::Ordering::SeqCst) {
            let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
            tokio::spawn({
                let app = app_state_clone.clone();
                let model = model_with_prefix.clone();
                let provider = provider_name.clone();
                let api_key = api_key_ref.clone();
                async move {
                    super::common::log_stream_success(
                        app,
                        start_time,
                        model,
                        provider,
                        api_key,
                        usage_snapshot,
                    )
                    .await;
                }
            });
        }

        es.close();
    });

    let out_stream = tokio_stream::StreamExt::map(
        tokio_stream::wrappers::UnboundedReceiverStream::new(rx),
        Ok::<_, Infallible>,
    );
    Ok(Sse::new(out_stream).keep_alive(axum::response::sse::KeepAlive::default()).into_response())
}
