use crate::error::GatewayError;

use super::types::{
    ChatCompletionRequest, ChatCompletionResponse, ModelListResponse, RawAndTypedChatCompletion,
};

pub struct OpenAIProvider;

impl OpenAIProvider {
    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

        let builder = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(request);

        // 非流式：优先严格解析；失败则宽松回退构造（兼容部分上游缺失 object 等字段）
        let response = builder.header("Accept", "application/json").send().await?;
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            // 上游返回空体，作为 JSON 解码失败处理
            return Err(GatewayError::Json(serde_json::Error::io(
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "empty body"),
            )));
        }
        let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
        let typed = match serde_json::from_slice::<ChatCompletionResponse>(&bytes) {
            Ok(ok) => ok,
            Err(_) => fallback_response_from_bytes(&bytes)?,
        };
        Ok(RawAndTypedChatCompletion { typed, raw })
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

    // 备注：流式聊天统一由 server/streaming 模块处理（基于 reqwest-eventsource）
}

#[allow(deprecated)]
fn fallback_response_from_bytes(bytes: &[u8]) -> Result<ChatCompletionResponse, GatewayError> {
    use async_openai::types as oai;
    let v: serde_json::Value = serde_json::from_slice(bytes)?;

    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let object = v
        .get("object")
        .and_then(|x| x.as_str())
        .unwrap_or("chat.completion")
        .to_string();
    let created = v
        .get("created")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32)
        .unwrap_or_else(|| chrono::Utc::now().timestamp() as u32);
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    // usage（宽松）
    let usage = v.get("usage").map(|u| oai::CompletionUsage {
        prompt_tokens: u
            .get("prompt_tokens")
            .and_then(|x| x.as_u64())
            .map(|x| x as u32)
            .unwrap_or(0),
        completion_tokens: u
            .get("completion_tokens")
            .and_then(|x| x.as_u64())
            .map(|x| x as u32)
            .unwrap_or(0),
        total_tokens: u
            .get("total_tokens")
            .and_then(|x| x.as_u64())
            .map(|x| x as u32)
            .unwrap_or(0),
        prompt_tokens_details: u
            .get("prompt_tokens_details")
            .map(|d| oai::PromptTokensDetails {
                cached_tokens: d
                    .get("cached_tokens")
                    .and_then(|x| x.as_u64())
                    .map(|x| x as u32),
                audio_tokens: None,
            }),
        completion_tokens_details: u.get("completion_tokens_details").map(|d| {
            oai::CompletionTokensDetails {
                reasoning_tokens: d
                    .get("reasoning_tokens")
                    .and_then(|x| x.as_u64())
                    .map(|x| x as u32),
                audio_tokens: None,
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }
        }),
    });

    // choices（尽力而为，保留 reasoning_content 等扩展字段）
    let mut choices: Vec<oai::ChatChoice> = Vec::new();
    if let Some(arr) = v.get("choices").and_then(|x| x.as_array()) {
        for (i, c) in arr.iter().enumerate() {
            let finish_reason =
                c.get("finish_reason")
                    .and_then(|x| x.as_str())
                    .and_then(|s| match s {
                        "stop" => Some(oai::FinishReason::Stop),
                        "length" => Some(oai::FinishReason::Length),
                        "tool_calls" => Some(oai::FinishReason::ToolCalls),
                        "content_filter" => Some(oai::FinishReason::ContentFilter),
                        _ => None,
                    });
            let msg = c
                .get("message")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let role = msg
                .get("role")
                .and_then(|x| x.as_str())
                .unwrap_or("assistant");
            let content = msg
                .get("content")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            // tool_calls（若存在）
            let tool_calls = msg
                .get("tool_calls")
                .and_then(|tc| tc.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|t| {
                            let id = t
                                .get("id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let f = t
                                .get("function")
                                .cloned()
                                .unwrap_or_else(|| serde_json::json!({}));
                            let name = f
                                .get("name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let arguments = f
                                .get("arguments")
                                .and_then(|x| x.as_str())
                                .unwrap_or("{}")
                                .to_string();
                            oai::ChatCompletionMessageToolCall {
                                id,
                                r#type: oai::ChatCompletionToolType::Function,
                                function: oai::FunctionCall { name, arguments },
                            }
                        })
                        .collect::<Vec<_>>()
                });

            let message = oai::ChatCompletionResponseMessage {
                role: match role {
                    "system" => oai::Role::System,
                    "user" => oai::Role::User,
                    "assistant" => oai::Role::Assistant,
                    _ => oai::Role::Assistant,
                },
                content,
                refusal: None,
                tool_calls,
                function_call: None,
                audio: None,
            };
            choices.push(oai::ChatChoice {
                index: c
                    .get("index")
                    .and_then(|x| x.as_u64())
                    .map(|x| x as u32)
                    .unwrap_or(i as u32),
                message,
                finish_reason,
                logprobs: None,
            });
        }
    }

    Ok(oai::CreateChatCompletionResponse {
        id,
        object,
        created,
        model,
        choices,
        usage,
        service_tier: None,
        system_fingerprint: None,
    })
}
