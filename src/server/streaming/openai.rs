use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::response::{IntoResponse, Response, Sse};
use chrono::{DateTime, Utc};
use reqwest_eventsource::{Event, RequestBuilderExt};

use async_openai::types::{ChatCompletionStreamOptions, CreateChatCompletionStreamResponse};
use serde_json::Value;

use crate::error::GatewayError;

fn join_openai_compat_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    let base_path = match reqwest::Url::parse(base) {
        Ok(u) => u.path().trim_end_matches('/').to_string(),
        Err(_) => String::new(),
    };

    if base_path.ends_with("/v1") || base_path.ends_with("/api/v3") {
        format!("{}/{}", base, normalized_path)
    } else {
        format!("{}/v1/{}", base, normalized_path)
    }
}
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::server::AppState;

use crate::server::util::mask_key;

/// 面向 OpenAI 兼容上游的流式聊天实现：
/// - 将请求改写为 SSE 流式接口并启用 usage 回传
/// - 持续消费 EventSource 事件，解析 usage 并通过 common 模块记录日志与计费
/// - 将原始 SSE 数据透传给网关调用方（含 [DONE] 事件与错误信息）
#[allow(clippy::too_many_arguments)]
pub async fn stream_openai_chat(
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
) -> Result<Response, GatewayError> {
    let url = join_openai_compat_endpoint(&base_url, "chat/completions");
    let client = crate::http_client::client_for_url(&url)?;

    upstream_req.stream = Some(true);
    upstream_req.stream_options = Some(ChatCompletionStreamOptions {
        include_usage: true,
    });

    let request_builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&upstream_req);

    let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
    let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 统计与日志关联使用稳定脱敏值，避免明文泄露
    let api_key_ref = Some(mask_key(&api_key));

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
    let usage_cell_for_task = usage_cell.clone();
    let app_state_clone = app_state.clone();
    let client_token_for_outer = client_token.clone();
    let _client_token_outer = client_token.clone();
    tokio::spawn(async move {
        let mut es = match request_builder.eventsource() {
            Ok(es) => es,
            Err(e) => {
                tracing::error!("Failed to open eventsource: {}", e);
                let state_for_log = app_state_clone.clone();
                let billing_model_for_log = model_with_prefix.clone();
                let requested_model_for_log = requested_model.clone();
                let effective_model_for_log = effective_model.clone();
                let provider_for_log = provider_name.clone();
                let api_key_for_log = api_key_ref.clone();
                let started_at = start_time;
                let msg = e.to_string();
                let ct_err = client_token_for_outer.clone();
                tokio::spawn(async move {
                    super::common::log_stream_error(
                        state_for_log,
                        started_at,
                        billing_model_for_log,
                        requested_model_for_log,
                        effective_model_for_log,
                        provider_for_log,
                        api_key_for_log,
                        ct_err,
                        msg,
                    )
                    .await;
                });
                let _ =
                    tx.send(axum::response::sse::Event::default().data(format!("error: {}", e)));
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
                            let ct_done = client_token_for_outer.clone();
                            tokio::spawn({
                                let app = app_state_clone.clone();
                                let billing_model = model_with_prefix.clone();
                                let requested_model = requested_model.clone();
                                let effective_model = effective_model.clone();
                                let provider = provider_name.clone();
                                let api_key = api_key_ref.clone();
                                async move {
                                    super::common::log_stream_success(
                                        app,
                                        start_time,
                                        billing_model,
                                        requested_model,
                                        effective_model,
                                        provider,
                                        api_key,
                                        ct_done,
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
                    if let Ok(chunk) =
                        serde_json::from_str::<CreateChatCompletionStreamResponse>(&m.data)
                        && let Some(u) = &chunk.usage
                    {
                        *usage_cell_for_task.lock().unwrap() = Some(u.clone());
                        captured = true;
                    }
                    // Fallback: Value parse to extract usage (tolerate vendor extensions)
                    if !captured
                        && let Ok(v) = serde_json::from_str::<Value>(&m.data)
                        && let Some(usage) = super::common::parse_usage_from_value(&v)
                    {
                        *usage_cell_for_task.lock().unwrap() = Some(usage);
                    }

                    let _ = tx.send(axum::response::sse::Event::default().data(m.data));
                }
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    let error_msg = e.to_string();
                    if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        let state_for_log = app_state_clone.clone();
                        let billing_model_for_log = model_with_prefix.clone();
                        let requested_model_for_log = requested_model.clone();
                        let effective_model_for_log = effective_model.clone();
                        let provider_for_log = provider_name.clone();
                        let api_key_for_log = api_key_ref.clone();
                        let started_at = start_time;
                        let error_for_log = error_msg.clone();
                        let ct_stream_err = client_token_for_outer.clone();
                        tokio::spawn(async move {
                            super::common::log_stream_error(
                                state_for_log,
                                started_at,
                                billing_model_for_log,
                                requested_model_for_log,
                                effective_model_for_log,
                                provider_for_log,
                                api_key_for_log,
                                ct_stream_err,
                                error_for_log,
                            )
                            .await;
                        });
                    }
                    let _ = tx.send(
                        axum::response::sse::Event::default().data(format!("error: {}", error_msg)),
                    );
                    break;
                }
            }
        }

        // Safety net: log if stream closed without [DONE]
        if !logged_flag.load(std::sync::atomic::Ordering::SeqCst) {
            let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
            let ct_fallback = client_token_for_outer.clone();
            tokio::spawn({
                let app = app_state_clone.clone();
                let billing_model = model_with_prefix.clone();
                let requested_model = requested_model.clone();
                let effective_model = effective_model.clone();
                let provider = provider_name.clone();
                let api_key = api_key_ref.clone();
                async move {
                    super::common::log_stream_success(
                        app,
                        start_time,
                        billing_model,
                        requested_model,
                        effective_model,
                        provider,
                        api_key,
                        ct_fallback,
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
    Ok(Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}
