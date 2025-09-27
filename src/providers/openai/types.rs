use serde::{Deserialize, Serialize};

// 将 Chat Completions 相关类型全面对齐 async-openai
pub use async_openai::types::{
    CreateChatCompletionRequest as ChatCompletionRequest,
    CreateChatCompletionResponse as ChatCompletionResponse,
};
pub use async_openai::types::CompletionUsage as Usage;

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
