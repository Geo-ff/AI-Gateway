use crate::error::GatewayError;

use super::types::{
    ChatCompletionRequest, ChatCompletionResponse, ModelListResponse,
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

        // 非流式对齐 ai-gateway：仅使用 JSON 响应
        let response = builder
            .header("Accept", "application/json")
            .send()
            .await?;
        Ok(response.json::<ChatCompletionResponse>().await?)
    }

    pub async fn list_models(
        base_url: &str,
        api_key: &str,
    ) -> Result<ModelListResponse, GatewayError> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/models", base_url.trim_end_matches('/'));

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .send()
            .await?;

        Ok(response.json::<ModelListResponse>().await?)
    }

    // 备注：流式聊天统一由 server/streaming_handlers.rs 处理（基于 reqwest-eventsource）
}
