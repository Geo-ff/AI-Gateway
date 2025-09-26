use axum::{
    response::{IntoResponse, Sse},
    extract::{State, Json},
    http::StatusCode,
};
use futures_util::StreamExt;
use std::{convert::Infallible, sync::{Arc, Mutex}};

use chrono::Utc;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::providers::streaming::StreamChatCompletionChunk;
use crate::server::AppState;
use crate::server::provider_dispatch::select_provider_for_model;
use crate::server::model_redirect::apply_model_redirects;
use crate::logging::RequestLog;

/// 处理流式聊天完成请求
pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // 如果用户没有请求流式传输，使用常规处理
    if !request.stream {
        return Err(StatusCode::BAD_REQUEST);
    }

    let start_time = Utc::now();

    apply_model_redirects(&mut request);

    // 使用基于模型的供应商选择逻辑
    let (selected, parsed_model) = select_provider_for_model(&app_state, &request.model)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    // 创建修改后的请求，使用实际模型名
    let mut modified_request = request.clone();
    modified_request.model = parsed_model.get_upstream_model_name().to_string();

    // 根据供应商类型创建流
    match selected.provider.api_type {
        crate::config::ProviderType::OpenAI => {
            // 直接在这里创建 OpenAI 流，避免生命周期问题
            let client = reqwest::Client::new();
            let url = format!("{}/v1/chat/completions", selected.provider.base_url.trim_end_matches('/'));

            // 创建流式请求
            let mut stream_request = modified_request.clone();
            stream_request.stream = true;

            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", selected.api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&stream_request)
                .send()
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

            // 用于记录 usage 和避免重复日志
            let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
            let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let app_state_clone = app_state.clone();
            let model_with_prefix = request.model.clone();
            let provider_name = selected.provider.name.clone();

            let stream = response
                .bytes_stream()
                .map(move |item| -> Result<axum::response::sse::Event, Infallible> {
                    match item {
                        Ok(bytes) => {
                            let text = String::from_utf8_lossy(&bytes);

                            // 解析 Server-Sent Events 格式
                            let mut first_event_data: Option<String> = None;
                            for line in text.lines() {
                                if line.starts_with("data: ") {
                                    let data = &line[6..]; // 去掉 "data: " 前缀

                                    if data == "[DONE]" {
                                        // 在流结束时记录日志（仅一次）
                                        if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                            let usage_snapshot = usage_cell.lock().unwrap().clone();
                                            let state_for_log = app_state_clone.clone();
                                            let model_for_log = model_with_prefix.clone();
                                            let provider_for_log = provider_name.clone();
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
                                                    model: Some(model_for_log),
                                                    provider: Some(provider_for_log),
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
                                        return Ok(axum::response::sse::Event::default().data("[DONE]"));
                                    }

                                    // 尝试从 JSON 数据中提取 usage
                                    if let Ok(chunk) = serde_json::from_str::<StreamChatCompletionChunk>(data) {
                                        if let Some(usage) = chunk.usage {
                                            *usage_cell.lock().unwrap() = Some(usage);
                                        }
                                    }

                                    if first_event_data.is_none() {
                                        first_event_data = Some(data.to_string());
                                    }
                                }
                            }

                            // 返回首个 data 事件；若没有则返回原始文本
                            if let Some(data) = first_event_data {
                                Ok(axum::response::sse::Event::default().data(data))
                            } else {
                                Ok(axum::response::sse::Event::default().data(text.to_string()))
                            }
                        }
                        Err(e) => {
                            tracing::error!("Stream error: {}", e);
                            // 出错时也尝试记录一次失败日志
                            if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                                let state_for_log = app_state_clone.clone();
                                let model_for_log = model_with_prefix.clone();
                                let provider_for_log = provider_name.clone();
                                let started_at = start_time;
                                tokio::spawn(async move {
                                    let end_time = Utc::now();
                                    let response_time_ms = (end_time - started_at).num_milliseconds();
                                    let log = RequestLog {
                                        id: None,
                                        timestamp: started_at,
                                        method: "POST".to_string(),
                                        path: "/v1/chat/completions".to_string(),
                                        model: Some(model_for_log),
                                        provider: Some(provider_for_log),
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
                            Ok(axum::response::sse::Event::default().data(format!("error: {}", e)))
                        }
                    }
                });

            Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
        }
        crate::config::ProviderType::Anthropic => {
            // Anthropic 流式传输暂未实现，返回错误
            Err(StatusCode::NOT_IMPLEMENTED)
        }
    }
}
