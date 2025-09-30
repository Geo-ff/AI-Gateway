use crate::error::GatewayError;
use crate::providers::openai::types::RawAndTypedChatCompletion;
use async_openai::types as oai;

// 轻量适配：
// - 去除 data:image/...;base64, 前缀，只保留逗号后的纯 base64 数据
// - 若 top_p >= 1，按 Newapi 适配压至 0.99，避免部分上游拒绝等边界
#[allow(clippy::collapsible_if)]
pub fn adapt_openai_request_for_zhipu(
    mut req: oai::CreateChatCompletionRequest,
) -> oai::CreateChatCompletionRequest {
    // 处理 top_p
    if let Some(tp) = req.top_p
        && tp >= 1.0
    {
        req.top_p = Some(0.99);
    }

    // 遍历消息，清洗 image_url 的 base64 前缀
    for msg in &mut req.messages {
        if let oai::ChatCompletionRequestMessage::User(m) = msg {
            if let oai::ChatCompletionRequestUserMessageContent::Array(parts) = &mut m.content {
                for part in parts.iter_mut() {
                    if let oai::ChatCompletionRequestUserMessageContentPart::ImageUrl(img) = part {
                        let url = &mut img.image_url.url;
                        if url.starts_with("data:image/")
                            && let Some(idx) = url.find(',')
                        {
                            let data = url[idx + 1..].to_string();
                            *url = data;
                        }
                    }
                }
            }
        }
    }

    req
}

pub async fn chat_completions(
    base_url: &str,
    api_key: &str,
    request: &oai::CreateChatCompletionRequest,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/paas/v4/chat/completions",
        base_url.trim_end_matches('/')
    );
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&adapt_openai_request_for_zhipu(request.clone()))
        .send()
        .await?;
    let bytes = resp.bytes().await?;
    if bytes.is_empty() {
        // 让 JSON 解码错误更直观地返回
        let _ = serde_json::from_slice::<serde_json::Value>(&bytes)?;
        unreachable!();
    }
    let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
    let typed = match serde_json::from_slice::<oai::CreateChatCompletionResponse>(&bytes) {
        Ok(ok) => ok,
        Err(_) => fallback_response_from_bytes(&bytes)?,
    };
    Ok(RawAndTypedChatCompletion { typed, raw })
}

#[allow(deprecated)]
fn fallback_response_from_bytes(
    bytes: &[u8],
) -> Result<oai::CreateChatCompletionResponse, GatewayError> {
    use async_openai::types as oai;
    let v: serde_json::Value = serde_json::from_slice(bytes)?;

    // 与 OpenAI 回退逻辑一致，尽力填充字段
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
                                .map(|x| match x {
                                    serde_json::Value::String(s) => s.clone(),
                                    _ => serde_json::to_string(x).unwrap_or("{}".to_string()),
                                })
                                .unwrap_or("{}".to_string());
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
