use serde::{Deserialize, Serialize};

// 将 Chat Completions 相关类型全面对齐 async-openai
pub use async_openai::types::CompletionUsage as Usage;
pub use async_openai::types::{
    CreateChatCompletionRequest as ChatCompletionRequest,
    CreateChatCompletionResponse as ChatCompletionResponse,
};

// 模型列表沿用本地定义（兼容多数上游返回）
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

// 非流式响应：同时保留原始 JSON（便于透传扩展字段，如 reasoning_content）与已解析的结构体供日志使用
#[derive(Debug, Clone)]
pub struct RawAndTypedChatCompletion {
    pub typed: ChatCompletionResponse,
    pub raw: serde_json::Value,
}
