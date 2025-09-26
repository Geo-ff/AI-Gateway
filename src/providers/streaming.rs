use serde::{Deserialize, Serialize};
use tokio_stream::Stream;
use std::pin::Pin;

/// 流式传输的 Delta 消息结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamMessage {
    pub role: Option<String>,
    pub content: Option<String>,
}

/// 流式传输的 Choice Delta
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoiceDelta {
    pub index: u32,
    pub delta: StreamMessage,
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub logprobs: Option<serde_json::Value>,
}

/// 流式传输的响应块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoiceDelta>,
    #[serde(default)]
    pub usage: Option<super::openai::Usage>,
}

/// Server-Sent Event 数据结构
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
}

impl SseEvent {
    pub fn new(data: String) -> Self {
        Self {
            id: None,
            event: Some("message".to_string()),
            data,
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_event(mut self, event: String) -> Self {
        self.event = Some(event);
        self
    }

    pub fn done() -> Self {
        Self {
            id: None,
            event: None,
            data: "[DONE]".to_string(),
        }
    }

    /// 格式化为 Server-Sent Event 格式
    pub fn format_sse(&self) -> String {
        let mut sse_data = String::new();

        if let Some(id) = &self.id {
            sse_data.push_str(&format!("id: {}\n", id));
        }

        if let Some(event) = &self.event {
            sse_data.push_str(&format!("event: {}\n", event));
        }

        // 处理多行数据
        for line in self.data.lines() {
            sse_data.push_str(&format!("data: {}\n", line));
        }

        sse_data.push('\n');
        sse_data
    }
}

/// 流式传输错误类型
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Stream processing error: {0}")]
    Stream(String),
}

/// 流式传输的结果类型
pub type StreamResult<T> = Result<T, StreamError>;

/// 流式传输的响应流类型
pub type ResponseStream = Pin<Box<dyn Stream<Item = StreamResult<SseEvent>> + Send>>;