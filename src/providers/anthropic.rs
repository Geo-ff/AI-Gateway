use serde::{Deserialize, Serialize};
use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, Message as OpenAIMessage, Choice, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub role: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub struct AnthropicProvider;

impl AnthropicProvider {
    pub fn convert_openai_to_anthropic(openai_req: &ChatCompletionRequest) -> AnthropicRequest {
        let messages = openai_req.messages
            .iter()
            .map(|msg| AnthropicMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            })
            .collect();

        AnthropicRequest {
            model: openai_req.model.clone(),
            max_tokens: openai_req.max_tokens.unwrap_or(1024),
            messages,
            temperature: openai_req.temperature,
            stream: if openai_req.stream { Some(true) } else { None },
        }
    }

    pub fn convert_anthropic_to_openai(anthropic_resp: &AnthropicResponse) -> ChatCompletionResponse {
        let content = anthropic_resp.content
            .first()
            .map(|block| block.text.clone())
            .unwrap_or_default();

        let choice = Choice {
            index: 0,
            message: OpenAIMessage {
                role: anthropic_resp.role.clone(),
                content,
            },
            finish_reason: anthropic_resp.stop_reason.clone(),
        };

        ChatCompletionResponse {
            id: anthropic_resp.id.clone(),
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp() as u64,
            model: anthropic_resp.model.clone(),
            choices: vec![choice],
            usage: Usage {
                prompt_tokens: anthropic_resp.usage.input_tokens,
                completion_tokens: anthropic_resp.usage.output_tokens,
                total_tokens: anthropic_resp.usage.input_tokens + anthropic_resp.usage.output_tokens,
            },
        }
    }

    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &AnthropicRequest,
    ) -> Result<AnthropicResponse, reqwest::Error> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));

        let response = client
            .post(&url)
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .json(request)
            .send()
            .await?;

        response.json::<AnthropicResponse>().await
    }
}