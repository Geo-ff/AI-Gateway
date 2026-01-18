use crate::providers::openai::ChatCompletionRequest;
use anthropic_ai_sdk::types::message as anthropic;
use async_openai::types as oai;

mod client;
mod request;
mod response;
mod utils;

pub struct AnthropicProvider;

impl AnthropicProvider {
    pub fn convert_openai_to_anthropic(
        openai_req: &ChatCompletionRequest,
    ) -> anthropic::CreateMessageParams {
        request::convert_openai_to_anthropic(openai_req, None)
    }

    pub fn convert_openai_to_anthropic_with_top_k(
        openai_req: &ChatCompletionRequest,
        top_k: Option<u32>,
    ) -> anthropic::CreateMessageParams {
        request::convert_openai_to_anthropic(openai_req, top_k)
    }

    pub fn convert_anthropic_to_openai(
        resp: &anthropic::CreateMessageResponse,
    ) -> oai::CreateChatCompletionResponse {
        response::convert_anthropic_to_openai(resp)
    }

    pub fn extract_reasoning_content(resp: &anthropic::CreateMessageResponse) -> Option<String> {
        response::extract_reasoning_content(resp)
    }

    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &anthropic::CreateMessageParams,
    ) -> crate::error::Result<anthropic::CreateMessageResponse> {
        client::chat_completions(base_url, api_key, request).await
    }
}
