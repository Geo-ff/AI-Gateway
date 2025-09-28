use crate::providers::openai::ChatCompletionRequest;
use async_openai::types as oai;
use anthropic_ai_sdk::types::message as anthropic;

mod request;
mod response;
mod utils;
mod client;

pub struct AnthropicProvider;

impl AnthropicProvider {
    pub fn convert_openai_to_anthropic(openai_req: &ChatCompletionRequest) -> anthropic::CreateMessageParams {
        request::convert_openai_to_anthropic(openai_req)
    }

    pub fn convert_anthropic_to_openai(resp: &anthropic::CreateMessageResponse) -> oai::CreateChatCompletionResponse {
        response::convert_anthropic_to_openai(resp)
    }

    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &anthropic::CreateMessageParams,
    ) -> crate::error::Result<anthropic::CreateMessageResponse> {
        client::chat_completions(base_url, api_key, request).await
    }
}

