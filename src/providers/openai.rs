use serde::{Deserialize, Serialize};
use futures_util::Stream;
use tokio_stream::StreamExt;
use crate::providers::streaming::{SseEvent, StreamChatCompletionChunk, StreamError, StreamResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    #[serde(default)]
    pub refs: Option<serde_json::Value>,
    #[serde(default)]
    pub logprobs: Option<LogProbs>,
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogProbs {
    // 这里可以根据需要扩展logprobs的具体结构
    #[serde(default)]
    pub content: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelListResponse {
    pub object: String,
    pub data: Vec<Model>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

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

        response.json::<ChatCompletionResponse>().await
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