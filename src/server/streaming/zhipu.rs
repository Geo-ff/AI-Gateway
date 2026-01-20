use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use axum::response::{IntoResponse, Response, Sse};
use chrono::{DateTime, Utc};
use reqwest_eventsource::{Event, RequestBuilderExt};
use serde_json::Value;

use crate::error::GatewayError;
use crate::providers::openai::{ChatCompletionRequest, Usage};
use crate::server::AppState;

use crate::server::util::mask_key;

/// 面向智谱 API 的流式聊天实现：
/// - 先将 OpenAI 风格请求适配为智谱专用格式（base64 清洗、top_p 调整等）
/// - 通过 SSE 消费上游流式响应，宽松提取 usage 并记录日志/计费
/// - 将原始 SSE 数据透传给网关调用方，保证与 OpenAI 路径一致的体验
#[allow(clippy::too_many_arguments)]
pub async fn stream_zhipu_chat(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model_with_prefix: String,
    requested_model: String,
    effective_model: String,
    base_url: String,
    provider_name: String,
    api_key: String,
    client_token: Option<String>,
    upstream_req: ChatCompletionRequest,
) -> Result<Response, GatewayError> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/paas/v4/chat/completions",
        base_url.trim_end_matches('/')
    );

    // 适配请求内容（base64 前缀清洗、top_p 修正）
    let adapted = crate::providers::zhipu::adapt_openai_request_for_zhipu(upstream_req);

    let request_builder = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(&adapted);

    let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
    let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // 统计与日志关联使用稳定脱敏值，避免明文泄露
    let api_key_ref = Some(mask_key(&api_key));

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
    let usage_cell_for_task = usage_cell.clone();
    let app_state_clone = app_state.clone();
    let client_token_for_outer = client_token.clone();
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

                    // 捕获 usage（Zhipu：宽松提取）
                    if let Ok(v) = serde_json::from_str::<Value>(&m.data)
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

        // 兜底：未收到 [DONE] 但流已结束，按最后一次 usage 记录日志
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
