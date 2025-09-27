use axum::{
    response::{IntoResponse, Sse},
    extract::{State, Json},
};
use futures_util::StreamExt;
use std::{convert::Infallible, sync::{Arc, Mutex}};
use reqwest_eventsource::{Event, RequestBuilderExt};

use chrono::Utc;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use async_openai::types::{ChatCompletionStreamOptions, CreateChatCompletionStreamResponse, CompletionTokensDetails, PromptTokensDetails};
use serde_json::Value;
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
    if !request.stream.unwrap_or(false) {
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
            stream_request.stream = Some(true);
            // 对齐 ai-gateway：明确请求在流式增量中返回 usage
            stream_request.stream_options = Some(ChatCompletionStreamOptions { include_usage: true });

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
            let usage_cell_for_task = usage_cell.clone();
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
                            if m.data.trim() == "[DONE]" {
                                if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                    let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                                    let state_for_log = app_state_clone.clone();
                                    let model_for_log = model_with_prefix.clone();
                                    let provider_for_log = provider_name.clone();
                                    let api_key_for_log = api_key_ref.clone();
                                    let started_at = start_time;
                                    tokio::spawn(async move {
                                        let end_time = Utc::now();
                                        let response_time_ms = (end_time - started_at).num_milliseconds();
                                        let (prompt, completion, total, cached, reasoning) = usage_snapshot
                                            .map(|u| (
                                                Some(u.prompt_tokens),
                                                Some(u.completion_tokens),
                                                Some(u.total_tokens),
                                                u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
                                                u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
                                            ))
                                            .unwrap_or((None, None, None, None, None));
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
                                            cached_tokens: cached,
                                            reasoning_tokens: reasoning,
                                            error_message: None,
                                        };
                                        if let Err(e) = state_for_log.log_store.log_request(log).await {
                                            tracing::error!("Failed to log streaming request: {}", e);
                                        }
                                    });
                                }
                                let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));
                                break;
                            }

                            // 捕获上游 usage（OpenAI 保持严格 typed 解析，避免引入回归）
                            if let Ok(chunk) = serde_json::from_str::<CreateChatCompletionStreamResponse>(&m.data) {
                                if let Some(u) = &chunk.usage {
                                    *usage_cell_for_task.lock().unwrap() = Some(u.clone());
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
                                        cached_tokens: None,
                                        reasoning_tokens: None,
                                        error_message: Some(error_for_log),
                                    };
                                    if let Err(e) = state_for_log.log_store.log_request(log).await {
                                        tracing::error!("Failed to log streaming error: {}", e);
                                    }
                                });
                            }
                            let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", error_msg)));
                            break;
                        }
                    }
                }

                // 流被动关闭但未收到 [DONE]，做一次兜底日志
                if !logged_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                    let state_for_log = app_state_clone.clone();
                    let model_for_log = model_with_prefix.clone();
                    let provider_for_log = provider_name.clone();
                    let api_key_for_log = api_key_ref.clone();
                    let started_at = start_time;
                    tokio::spawn(async move {
                        let end_time = Utc::now();
                        let response_time_ms = (end_time - started_at).num_milliseconds();
                        let (prompt, completion, total, cached, reasoning) = usage_snapshot
                            .map(|u| (
                                Some(u.prompt_tokens),
                                Some(u.completion_tokens),
                                Some(u.total_tokens),
                                u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
                                u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
                            ))
                            .unwrap_or((None, None, None, None, None));
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
                            cached_tokens: cached,
                            reasoning_tokens: reasoning,
                            error_message: None,
                        };
                        if let Err(e) = state_for_log.log_store.log_request(log).await {
                            tracing::error!("Failed to log streaming request on close: {}", e);
                        }
                    });
                }

                es.close();
            });

            let out_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
                .map(Ok::<_, Infallible>);
            Ok(Sse::new(out_stream).keep_alive(axum::response::sse::KeepAlive::default()))
        }
        crate::config::ProviderType::Anthropic => Err(GatewayError::Config("Anthropic streaming not implemented".into())),
        crate::config::ProviderType::Zhipu => {
            // 与 OpenAI 分支类似，但使用 Zhipu 专用路径与请求适配
            let client = reqwest::Client::new();
            let url = format!("{}/api/paas/v4/chat/completions", selected.provider.base_url.trim_end_matches('/'));

            let mut stream_request = modified_request.clone();
            stream_request.stream = Some(true);
            // Zhipu 不依赖 include_usage 开关，这里不设置 stream_options

            // 适配请求内容（base64 前缀清洗、top_p 修正）
            let adapted = crate::providers::zhipu::adapt_openai_request_for_zhipu(stream_request);

            let request_builder = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", selected.api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&adapted);

            let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
            let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let app_state_clone = app_state.clone();
            let model_with_prefix = request.model.clone();
            let provider_name = selected.provider.name.clone();
            let api_key_ref = api_key_hint(&app_state.config.logging, &selected.api_key);

            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
            let usage_cell_for_task = usage_cell.clone();
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
                        Ok(Event::Open) => {}
                        Ok(Event::Message(m)) => {
                            if m.data.trim() == "[DONE]" {
                                if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                    let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                                    let state_for_log = app_state_clone.clone();
                                    let model_for_log = model_with_prefix.clone();
                                    let provider_for_log = provider_name.clone();
                                    let api_key_for_log = api_key_ref.clone();
                                    let started_at = start_time;
                                    tokio::spawn(async move {
                                        let end_time = Utc::now();
                                        let response_time_ms = (end_time - started_at).num_milliseconds();
                                        let (prompt, completion, total, cached, reasoning) = usage_snapshot
                                            .map(|u| (
                                                Some(u.prompt_tokens),
                                                Some(u.completion_tokens),
                                                Some(u.total_tokens),
                                                u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
                                                u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
                                            ))
                                            .unwrap_or((None, None, None, None, None));
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
                                            cached_tokens: cached,
                                            reasoning_tokens: reasoning,
                                            error_message: None,
                                        };
                                        if let Err(e) = state_for_log.log_store.log_request(log).await {
                                            tracing::error!("Failed to log streaming request: {}", e);
                                        }
                                    });
                                }
                                let _ = tx.send(axum::response::sse::Event::default().data("[DONE]"));
                                break;
                            }

                            // 捕获 usage（Zhipu：宽松提取）
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
                                        cached_tokens: None,
                                        reasoning_tokens: None,
                                        error_message: Some(error_for_log),
                                    };
                                    if let Err(e) = state_for_log.log_store.log_request(log).await {
                                        tracing::error!("Failed to log streaming error: {}", e);
                                    }
                                });
                            }
                            let _ = tx.send(axum::response::sse::Event::default().data(format!("error: {}", error_msg)));
                            break;
                        }
                    }
                }

                // 兜底：未收到 [DONE] 但流已结束，按最后一次 usage 记录日志
                if !logged_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                    let state_for_log = app_state_clone.clone();
                    let model_for_log = model_with_prefix.clone();
                    let provider_for_log = provider_name.clone();
                    let api_key_for_log = api_key_ref.clone();
                    let started_at = start_time;
                    tokio::spawn(async move {
                        let end_time = Utc::now();
                        let response_time_ms = (end_time - started_at).num_milliseconds();
                        let (prompt, completion, total, cached, reasoning) = usage_snapshot
                            .map(|u| (
                                Some(u.prompt_tokens),
                                Some(u.completion_tokens),
                                Some(u.total_tokens),
                                u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
                                u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
                            ))
                            .unwrap_or((None, None, None, None, None));
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
                            cached_tokens: cached,
                            reasoning_tokens: reasoning,
                            error_message: None,
                        };
                        if let Err(e) = state_for_log.log_store.log_request(log).await {
                            tracing::error!("Failed to log streaming request on close: {}", e);
                        }
                    });
                }

                es.close();
            });

            let out_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
                .map(Ok::<_, Infallible>);
            Ok(Sse::new(out_stream).keep_alive(axum::response::sse::KeepAlive::default()))
        }
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
