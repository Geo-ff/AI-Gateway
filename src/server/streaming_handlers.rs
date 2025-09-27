use axum::{
    response::{IntoResponse, Sse},
    extract::{State, Json},
};
use futures_util::StreamExt;
use std::{convert::Infallible, sync::{Arc, Mutex}};
use reqwest_eventsource::{Event, RequestBuilderExt};

use chrono::Utc;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::providers::streaming::StreamChatCompletionChunk;
use crate::server::AppState;
use crate::server::provider_dispatch::select_provider_for_model;
use crate::server::model_redirect::apply_model_redirects;
use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_CHAT_STREAM;
use crate::error::GatewayError;
use crate::config::settings::{KeyLogStrategy, LoggingConfig};

/// 处理流式聊天完成请求
pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, GatewayError> {
    // 如果用户没有请求流式传输，使用常规处理
    if !request.stream {
        return Err(GatewayError::Config("stream=false for streaming endpoint".into()));
    }

    let start_time = Utc::now();

    apply_model_redirects(&mut request);

    // 使用基于模型的供应商选择逻辑
    let (selected, parsed_model) = select_provider_for_model(&app_state, &request.model).await?;

    // 创建修改后的请求，使用实际模型名
    let mut modified_request = request.clone();
    modified_request.model = parsed_model.get_upstream_model_name().to_string();

    // 根据供应商类型创建流
    match selected.provider.api_type {
        crate::config::ProviderType::OpenAI => {
            // 使用 reqwest-eventsource 建立稳定的 SSE 事件源
            let client = reqwest::Client::new();
            let url = format!("{}/v1/chat/completions", selected.provider.base_url.trim_end_matches('/'));

            let mut stream_request = modified_request.clone();
            stream_request.stream = true;

            let request_builder = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", selected.api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&stream_request);

            // 用于记录 usage 和避免重复日志
            let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
            let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let app_state_clone = app_state.clone();
            let model_with_prefix = request.model.clone();
            let provider_name = selected.provider.name.clone();

            // 准备 api_key 日志值（按策略处理）
            let api_key_ref = api_key_hint(&app_state.config.logging, &selected.api_key);

            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
            tokio::spawn(async move {
                let mut es = match request_builder.eventsource() {
                    Ok(es) => es,
                    Err(e) => {
                        tracing::error!("Failed to open eventsource: {}", e);
                        let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", e)));
                        return;
                    }
                };

                while let Some(ev) = es.next().await {
                    match ev {
                        Ok(Event::Open) => {
                            // ignore
                        }
                        Ok(Event::Message(m)) => {
                            if m.data == "[DONE]" {
                                if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                    let usage_snapshot = usage_cell.lock().unwrap().clone();
                                    let state_for_log = app_state_clone.clone();
                                    let model_for_log = model_with_prefix.clone();
                                    let provider_for_log = provider_name.clone();
                                    let api_key_for_log = api_key_ref.clone();
                                    let started_at = start_time;
                                    tokio::spawn(async move {
                                        let end_time = Utc::now();
                                        let response_time_ms = (end_time - started_at).num_milliseconds();
                                        let (prompt, completion, total) = usage_snapshot
                                            .map(|u| (Some(u.prompt_tokens), Some(u.completion_tokens), Some(u.total_tokens)))
                                            .unwrap_or((None, None, None));

                                        let log = RequestLog {
                                            id: None,
                                            timestamp: started_at,
                                            method: "POST".to_string(),
                                            path: "/v1/chat/completions".to_string(),
                                            request_type: REQ_TYPE_CHAT_STREAM.to_string(),
                                            model: Some(model_for_log),
                                            provider: Some(provider_for_log),
                                            api_key: api_key_for_log,
                                            status_code: 200,
                                            response_time_ms,
                                            prompt_tokens: prompt,
                                            completion_tokens: completion,
                                            total_tokens: total,
                                        };
                                        if let Err(e) = state_for_log.log_store.log_request(log).await {
                                            tracing::error!("Failed to log streaming request: {}", e);
                                        }
                                    });
                                }
                                let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));
                                break;
                            }

                            // 尝试从事件 JSON 捕获 usage
                            if let Ok(chunk) = serde_json::from_str::<StreamChatCompletionChunk>(&m.data) {
                                if let Some(u) = &chunk.usage {
                                    *usage_cell.lock().unwrap() = Some(u.clone());
                                }
                            }

                            let _ = tx.send(axum::response::sse::Event::default().data(m.data));
                        }
                        Err(e) => {
                            tracing::error!("Stream error: {}", e);
                            if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                let state_for_log = app_state_clone.clone();
                                let model_for_log = model_with_prefix.clone();
                                let provider_for_log = provider_name.clone();
                                let api_key_for_log = api_key_ref.clone();
                                let started_at = start_time;
                                tokio::spawn(async move {
                                    let end_time = Utc::now();
                                    let response_time_ms = (end_time - started_at).num_milliseconds();
                                    let log = RequestLog {
                                        id: None,
                                        timestamp: started_at,
                                        method: "POST".to_string(),
                                        path: "/v1/chat/completions".to_string(),
                                        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
                                        model: Some(model_for_log),
                                        provider: Some(provider_for_log),
                                        api_key: api_key_for_log,
                                        status_code: 500,
                                        response_time_ms,
                                        prompt_tokens: None,
                                        completion_tokens: None,
                                        total_tokens: None,
                                    };
                                    if let Err(e) = state_for_log.log_store.log_request(log).await {
                                        tracing::error!("Failed to log streaming error: {}", e);
                                    }
                                });
                            }
                            let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", e)));
                            break;
                        }
                    }
                }

                es.close();
            });

            let out_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
                .map(Ok::<_, Infallible>);
            Ok(Sse::new(out_stream).keep_alive(axum::response::sse::KeepAlive::default()))
        }
        crate::config::ProviderType::Anthropic => Err(GatewayError::Config("Anthropic streaming not implemented".into())),
    }
}

fn api_key_hint(cfg: &LoggingConfig, key: &str) -> Option<String> {
    match cfg.key_log_strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 { return "****".to_string(); }
    let (start, end) = (&key[..4], &key[key.len()-4..]);
    format!("{}****{}", start, end)
}
