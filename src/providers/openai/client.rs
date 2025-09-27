use chrono::Utc;
use futures_util::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use uuid::Uuid;

use crate::error::GatewayError;
use crate::providers::streaming::StreamChatCompletionChunk;

use super::types::{
    ChatCompletionRequest, ChatCompletionResponse, Choice, Message, ModelListResponse, Usage,
};

pub struct OpenAIProvider;

impl OpenAIProvider {
    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, GatewayError> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

        let builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(request);

        // 优先尝试以 SSE 打开，兼容上游在非流式下仍返回 text/event-stream 的场景
        if let (Some(b_sse), Some(b_json)) = (builder.try_clone(), builder.try_clone()) {
            // SSE 尝试
            match b_sse
                .header("Accept", "text/event-stream")
                .eventsource()
            {
                Ok(mut es) => {
                    let mut full_content = String::new();
                    let mut usage: Option<Usage> = None;
                    let mut model_name: Option<String> = None;
                    let mut final_role: String = "assistant".to_string();
                    let mut final_id: Option<String> = None;

                    while let Some(ev) = es.next().await {
                        match ev {
                            Ok(Event::Open) => {}
                            Ok(Event::Message(m)) => {
                                if m.data == "[DONE]" {
                                    break;
                                }
                                if let Ok(chunk) = serde_json::from_str::<StreamChatCompletionChunk>(&m.data) {
                                    if model_name.is_none() { model_name = Some(chunk.model.clone()); }
                                    if final_id.is_none() { final_id = Some(chunk.id.clone()); }
                                    if let Some(u) = chunk.usage.clone() { usage = Some(u); }
                                    if let Some(choice) = chunk.choices.first() {
                                        if let Some(role) = &choice.delta.role { final_role = role.clone(); }
                                        if let Some(delta) = &choice.delta.content { full_content.push_str(delta); }
                                    }
                                } else {
                                    // 非标准 JSON 片段：保留不丢字
                                    full_content.push_str(&m.data);
                                }
                            }
                            Err(_e) => {
                                // SSE 打开但中途错误：退出并以当前聚合为准
                                break;
                            }
                        }
                    }
                    es.close();

                    // 若在 SSE 中聚合到任何内容/元数据，则返回一次性响应
                    if !full_content.is_empty() || usage.is_some() || model_name.is_some() || final_id.is_some() {
                        let usage = usage.unwrap_or(Usage {
                            prompt_tokens: 0,
                            completion_tokens: 0,
                            total_tokens: 0,
                            prompt_tokens_details: None,
                            completion_tokens_details: None,
                        });

                        let resp = ChatCompletionResponse {
                            id: final_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                            object: "chat.completion".to_string(),
                            created: Utc::now().timestamp() as u64,
                            model: model_name.unwrap_or_else(|| request.model.clone()),
                            choices: vec![Choice {
                                index: 0,
                                message: Message { role: final_role, content: full_content },
                                refs: None,
                                logprobs: None,
                                finish_reason: Some("stop".to_string()),
                                service_tier: None,
                            }],
                            usage,
                        };
                        return Ok(resp);
                    }

                    // 若打开了 SSE 但没有任何有效数据，回退到 JSON
                    let response = b_json
                        .header("Accept", "application/json")
                        .send()
                        .await?;
                    return Ok(response.json::<ChatCompletionResponse>().await?);
                }
                Err(_e) => {
                    // 不是 SSE（或不支持 SSE）：按 JSON 处理
                    let response = b_json
                        .header("Accept", "application/json")
                        .send()
                        .await?;
                    return Ok(response.json::<ChatCompletionResponse>().await?);
                }
            }
        }

        // 极端情况下无法 clone builder，直接按 JSON 发送
        let response = builder
            .header("Accept", "application/json")
            .send()
            .await?;
        Ok(response.json::<ChatCompletionResponse>().await?)
    }

    pub async fn list_models(
        base_url: &str,
        api_key: &str,
    ) -> Result<ModelListResponse, reqwest::Error> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/models", base_url.trim_end_matches('/'));

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        response.json::<ModelListResponse>().await
    }

    // 备注：流式聊天统一由 server/streaming_handlers.rs 处理（基于 reqwest-eventsource）
}
