use futures_util::Stream;
use tokio_stream::StreamExt;
use chrono::Utc;
use uuid::Uuid;

use crate::providers::streaming::{SseEvent, StreamChatCompletionChunk, StreamError, StreamResult};

use super::types::{
    ChatCompletionRequest, ChatCompletionResponse, Choice, Message, Model, ModelListResponse, Usage,
};

pub struct OpenAIProvider;

impl OpenAIProvider {
    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, reqwest::Error> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await?;

        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.contains("text/event-stream"))
            .unwrap_or(false);

        if !is_sse {
            return response.json::<ChatCompletionResponse>().await;
        }

        // 上游意外返回SSE：聚合为一次性响应，保持非流式调用语义
        let mut full_content = String::new();
        let mut usage: Option<Usage> = None;
        let mut model_name: Option<String> = None;
        let mut final_role: String = "assistant".to_string();
        let mut final_id: Option<String> = None;

        let mut stream = response.bytes_stream();
        while let Some(item) = stream.next().await {
            let bytes = match item { Ok(b) => b, Err(e) => return Err(e) };
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                if !line.starts_with("data: ") { continue; }
                let data = &line[6..];
                if data == "[DONE]" { continue; }
                if let Ok(chunk) = serde_json::from_str::<StreamChatCompletionChunk>(data) {
                    if model_name.is_none() { model_name = Some(chunk.model.clone()); }
                    if final_id.is_none() { final_id = Some(chunk.id.clone()); }
                    if let Some(u) = chunk.usage.clone() { usage = Some(u); }
                    if let Some(choice) = chunk.choices.first() {
                        if let Some(role) = &choice.delta.role { final_role = role.clone(); }
                        if let Some(delta) = &choice.delta.content { full_content.push_str(delta); }
                    }
                }
            }
        }

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

        Ok(resp)
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

    /// 流式聊天完成
    pub async fn chat_completions_stream(
        base_url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<impl Stream<Item = StreamResult<SseEvent>>, reqwest::Error> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

        // 创建流式请求
        let mut stream_request = request.clone();
        stream_request.stream = true;

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&stream_request)
            .send()
            .await?;

        let stream = response
            .bytes_stream()
            .map(|item| -> StreamResult<SseEvent> {
                let bytes = item.map_err(StreamError::Http)?;
                let text = String::from_utf8_lossy(&bytes);

                // 解析 Server-Sent Events 格式
                let mut buffer = String::new();
                for line in text.lines() {
                    if line.starts_with("data: ") {
                        let data = &line[6..]; // 去掉 "data: " 前缀

                        if data == "[DONE]" {
                            return Ok(SseEvent::done());
                        }

                        // 尝试解析 JSON 数据
                        match serde_json::from_str::<StreamChatCompletionChunk>(data) {
                            Ok(chunk) => {
                                let json_data = serde_json::to_string(&chunk)
                                    .map_err(StreamError::Json)?;
                                return Ok(SseEvent::new(json_data));
                            }
                            Err(e) => {
                                // 如果解析失败，直接传递原始数据
                                tracing::warn!("Failed to parse streaming chunk: {}", e);
                                return Ok(SseEvent::new(data.to_string()));
                            }
                        }
                    } else if !line.trim().is_empty() {
                        buffer.push_str(line);
                        buffer.push('\n');
                    }
                }

                // 如果有缓冲的数据，返回它
                if !buffer.is_empty() {
                    Ok(SseEvent::new(buffer))
                } else {
                    // 否则返回原始文本
                    Ok(SseEvent::new(text.to_string()))
                }
            });

        Ok(stream)
    }
}

