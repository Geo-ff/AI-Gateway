use crate::error::GatewayError;
use crate::providers::adapters::gateway_error_from_normalized;

use super::types::{
    ChatCompletionRequest, ChatCompletionResponse, ModelListResponse, RawAndTypedChatCompletion,
};

pub struct OpenAIProvider;

fn join_openai_compat_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    let base_path = match reqwest::Url::parse(base) {
        Ok(u) => u.path().trim_end_matches('/').to_string(),
        Err(_) => String::new(),
    };

    if base_path.ends_with("/v1") || base_path.ends_with("/api/v3") {
        format!("{}/{}", base, normalized_path)
    } else {
        format!("{}/v1/{}", base, normalized_path)
    }
}

impl OpenAIProvider {
    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &ChatCompletionRequest,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let url = join_openai_compat_endpoint(base_url, "chat/completions");
        let client = crate::http_client::client_for_url(&url)?;

        async fn send_bytes(
            client: &reqwest::Client,
            url: &str,
            api_key: &str,
            request: &ChatCompletionRequest,
        ) -> Result<Vec<u8>, GatewayError> {
            let response = client
                .post(url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(request)
                .send()
                .await?;
            Ok(response.bytes().await?.to_vec())
        }

        fn parse_non_stream_bytes(bytes: &[u8]) -> Result<RawAndTypedChatCompletion, GatewayError> {
            if bytes.is_empty() || bytes.iter().all(|b| b.is_ascii_whitespace()) {
                return Err(GatewayError::Json(serde_json::Error::io(
                    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "empty body"),
                )));
            }

            let normalized = normalize_non_stream_response_bytes(bytes)?;
            let raw: serde_json::Value = serde_json::from_slice(&normalized).or_else(|_| {
                // best-effort: some upstreams always return SSE "data: ..." even when stream=false
                let aggregated = sse_chat_completion_to_json(&normalized)?;
                Ok::<serde_json::Value, GatewayError>(serde_json::from_slice(&aggregated)?)
            })?;

            let typed = match serde_json::from_slice::<ChatCompletionResponse>(&normalized) {
                Ok(ok) => ok,
                Err(_) => {
                    // keep typed usable for logs; retry with SSE-aggregated JSON if needed
                    match fallback_response_from_bytes(&normalized) {
                        Ok(ok) => ok,
                        Err(_) => {
                            let aggregated = sse_chat_completion_to_json(&normalized)?;
                            match serde_json::from_slice::<ChatCompletionResponse>(&aggregated) {
                                Ok(ok) => ok,
                                Err(_) => fallback_response_from_bytes(&aggregated)?,
                            }
                        }
                    }
                }
            };
            if let Some(err) = gateway_error_from_openai_payload(&raw) {
                return Err(err);
            }
            Ok(RawAndTypedChatCompletion { typed, raw })
        }

        fn is_retryable_stream_required_error(raw: &serde_json::Value) -> bool {
            let Some(err) = raw.get("error") else {
                return false;
            };
            let code = err.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            matches!(code, "bad_response_body" | "bad_response_status_code")
                || msg.contains("bad_response_body")
                || msg.contains("bad_response_status_code")
        }

        fn should_retry_empty_success_response(dual: &RawAndTypedChatCompletion) -> bool {
            let completion_tokens = dual
                .typed
                .usage
                .as_ref()
                .map(|usage| usage.completion_tokens)
                .or_else(|| {
                    dual.raw
                        .get("usage")
                        .and_then(|usage| usage.get("completion_tokens"))
                        .and_then(|value| value.as_u64())
                        .map(|value| value as u32)
                })
                .unwrap_or(0);

            if completion_tokens == 0 {
                return false;
            }

            let typed_content = dual
                .typed
                .choices
                .first()
                .and_then(|choice| choice.message.content.as_deref())
                .map(str::trim)
                .filter(|content| !content.is_empty());
            if typed_content.is_some() {
                return false;
            }

            let raw_content = dual
                .raw
                .get("choices")
                .and_then(|choices| choices.as_array())
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"));

            match raw_content {
                Some(serde_json::Value::String(text)) => text.trim().is_empty(),
                Some(serde_json::Value::Null) | None => true,
                Some(serde_json::Value::Array(items)) => items.is_empty(),
                Some(serde_json::Value::Object(map)) => map.is_empty(),
                Some(_) => false,
            }
        }

        // 非流式：优先严格解析；失败则宽松回退构造（兼容部分上游缺失 object 等字段）。
        // 若上游聚合器对特定模型仅支持 stream=true，会返回结构化错误（bad_response_body 等），此时自动重试一次 stream=true，
        // 并将 SSE 聚合为非流式 JSON 返回给前端（对前端保持一次性响应语义）。
        let bytes = send_bytes(&client, &url, api_key, request).await?;
        let mut dual = parse_non_stream_bytes(&bytes)?;
        if !request.stream.unwrap_or(false)
            && (is_retryable_stream_required_error(&dual.raw)
                || should_retry_empty_success_response(&dual))
        {
            let mut streaming_req = request.clone();
            streaming_req.stream = Some(true);
            let bytes2 = send_bytes(&client, &url, api_key, &streaming_req).await?;
            dual = parse_non_stream_bytes(&bytes2)?;
        }
        Ok(dual)
    }

    pub async fn list_models(
        base_url: &str,
        api_key: &str,
    ) -> Result<ModelListResponse, GatewayError> {
        let url = join_openai_compat_endpoint(base_url, "models");
        let client = crate::http_client::client_for_url(&url)?;

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

fn normalize_non_stream_response_bytes(bytes: &[u8]) -> Result<Vec<u8>, GatewayError> {
    // 部分上游即使 stream=false 也会返回 SSE（data: ...），这里做 best-effort 兼容：
    // - 将 SSE chunks 聚合为一个非流式 chat.completion JSON
    // - 避免前端收到 bad_response_body
    let trimmed = bytes
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .copied()
        .collect::<Vec<u8>>();
    if trimmed.starts_with(b"data:")
        || trimmed.starts_with(b"event:")
        || trimmed.starts_with(b"id:")
        || trimmed.windows(6).take(256).any(|w| w == b"\ndata:")
    {
        return sse_chat_completion_to_json(bytes);
    }
    Ok(bytes.to_vec())
}

fn sse_chat_completion_to_json(bytes: &[u8]) -> Result<Vec<u8>, GatewayError> {
    use serde_json::{Value, json};
    let s = std::str::from_utf8(bytes).map_err(|e| GatewayError::Config(e.to_string()))?;
    let s = s.replace("\r\n", "\n");

    let mut id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut created: Option<u64> = None;
    let mut role: Option<String> = None;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<Value> = None;

    fn collect_text_fragments(value: &Value) -> Vec<String> {
        match value {
            Value::Null => Vec::new(),
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    Vec::new()
                } else {
                    vec![trimmed.to_string()]
                }
            }
            Value::Array(items) => items.iter().flat_map(collect_text_fragments).collect(),
            Value::Object(map) => {
                for key in ["delta", "text", "output_text", "value", "content"] {
                    if let Some(found) = map.get(key) {
                        let fragments = collect_text_fragments(found);
                        if !fragments.is_empty() {
                            return fragments;
                        }
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn append_fragments(target: &mut String, fragments: Vec<String>) {
        for fragment in fragments {
            if fragment.is_empty() {
                continue;
            }
            if !target.is_empty() {
                target.push_str("\n\n");
            }
            target.push_str(&fragment);
        }
    }

    fn append_stream_fragments(target: &mut String, fragments: Vec<String>) {
        for fragment in fragments {
            if fragment.is_empty() {
                continue;
            }
            target.push_str(&fragment);
        }
    }

    for line in s.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let data = rest.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }

        let v: Value = serde_json::from_str(data)?;
        if id.is_none() {
            id = v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        if model.is_none() {
            model = v
                .get("model")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
        }
        if created.is_none() {
            created = v.get("created").and_then(|x| x.as_u64());
        }
        if usage.is_none() {
            usage = v.get("usage").cloned();
        }
        if usage.is_none() {
            usage = v.get("response").and_then(|response| response.get("usage")).cloned();
        }

        if v.get("object").and_then(|x| x.as_str()) == Some("response") {
            append_fragments(&mut content, collect_text_fragments(&v));
            if let Some(response) = v.get("response") {
                if id.is_none() {
                    id = response
                        .get("id")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if model.is_none() {
                    model = response
                        .get("model")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if created.is_none() {
                    created = response.get("created").and_then(|x| x.as_u64());
                }
                if usage.is_none() {
                    usage = response.get("usage").cloned();
                }
            }
        }

        if let Some(event_type) = v.get("type").and_then(|x| x.as_str())
            && (event_type.starts_with("response.output_text")
                || event_type.starts_with("response.content_part")
                || event_type.starts_with("response.output_item")
                || event_type == "response.completed")
        {
            if event_type.contains(".delta") {
                append_stream_fragments(&mut content, collect_text_fragments(&v));
            } else {
                append_fragments(&mut content, collect_text_fragments(&v));
            }
            if let Some(response) = v.get("response") {
                if id.is_none() {
                    id = response
                        .get("id")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if model.is_none() {
                    model = response
                        .get("model")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if created.is_none() {
                    created = response.get("created").and_then(|x| x.as_u64());
                }
                if usage.is_none() {
                    usage = response.get("usage").cloned();
                }
            }
        }

        // OpenAI streaming: choices[].delta
        if let Some(choice0) = v
            .get("choices")
            .and_then(|x| x.as_array())
            .and_then(|arr| arr.first())
        {
            if let Some(fr) = choice0.get("finish_reason").and_then(|x| x.as_str()) {
                finish_reason = Some(fr.to_string());
            }

            if let Some(delta) = choice0.get("delta").and_then(|x| x.as_object()) {
                if role.is_none() {
                    role = delta
                        .get("role")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if let Some(delta_content) = delta.get("content") {
                    append_stream_fragments(&mut content, collect_text_fragments(delta_content));
                }
                if let Some(r) = delta
                    .get("reasoning_content")
                    .and_then(|x| x.as_str())
                    .or_else(|| delta.get("reasoning").and_then(|x| x.as_str()))
                    .or_else(|| delta.get("thinking").and_then(|x| x.as_str()))
                {
                    reasoning.push_str(r);
                }
            }

            // 一些上游可能直接返回 choices[].message（但仍使用 SSE 包装）
            if let Some(message) = choice0.get("message").and_then(|x| x.as_object()) {
                if role.is_none() {
                    role = message
                        .get("role")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                }
                if content.is_empty()
                    && let Some(message_content) = message.get("content")
                {
                    append_fragments(&mut content, collect_text_fragments(message_content));
                }
                if reasoning.is_empty() {
                    if let Some(r) = message
                        .get("reasoning_content")
                        .and_then(|x| x.as_str())
                        .or_else(|| message.get("reasoning").and_then(|x| x.as_str()))
                        .or_else(|| message.get("thinking").and_then(|x| x.as_str()))
                    {
                        reasoning = r.to_string();
                    }
                }
            }
        }
    }

    let mut message = json!({
        "role": role.unwrap_or_else(|| "assistant".to_string()),
        "content": content,
    });
    if !reasoning.trim().is_empty() {
        if let Some(obj) = message.as_object_mut() {
            obj.insert("reasoning_content".to_string(), Value::String(reasoning));
        }
    }

    let mut out = json!({
        "id": id.unwrap_or_else(|| format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis())),
        "object": "chat.completion",
        "created": created.unwrap_or_else(|| chrono::Utc::now().timestamp() as u64),
        "model": model.unwrap_or_default(),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason.unwrap_or_else(|| "stop".to_string()),
        }],
    });
    if let Some(u) = usage {
        if let Some(obj) = out.as_object_mut() {
            obj.insert("usage".to_string(), u);
        }
    }

    Ok(serde_json::to_vec(&out)?)
}

fn gateway_error_from_openai_payload(raw: &serde_json::Value) -> Option<GatewayError> {
    let error = raw.get("error")?;
    if raw.get("choices").is_some() {
        return None;
    }
    let error_type = error
        .get("type")
        .and_then(|value| value.as_str())
        .map(|value| match value {
            "invalid_api_key" | "authentication_error" => "authentication_failed",
            "rate_limit_error" => "rate_limited",
            _ => "upstream_error",
        })
        .unwrap_or("upstream_error");
    let message = error
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string());
    Some(gateway_error_from_normalized(error_type, message))
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
            let content = msg.get("content").and_then(|value| {
                match value {
                    serde_json::Value::String(text) => Some(text.clone()),
                    serde_json::Value::Array(parts) => {
                        let mut out = Vec::new();
                        for part in parts {
                            if let Some(text) = part
                                .get("text")
                                .and_then(|text| text.as_str())
                                .or_else(|| part.get("content").and_then(|text| text.as_str()))
                            {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    out.push(trimmed.to_string());
                                }
                            }
                        }
                        if out.is_empty() {
                            None
                        } else {
                            Some(out.join("\n\n"))
                        }
                    }
                    serde_json::Value::Object(object) => object
                        .get("text")
                        .and_then(|text| text.as_str())
                        .or_else(|| object.get("content").and_then(|text| text.as_str()))
                        .map(str::to_string),
                    _ => None,
                }
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn sse_is_aggregated_into_non_stream_response() {
        let sse = b"data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"he\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n";
        let out = sse_chat_completion_to_json(sse).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["choices"][0]["message"]["content"], "hello");
    }

    #[test]
    fn responses_sse_is_aggregated_into_non_stream_response() {
        let sse = b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\",\"created\":1,\"model\":\"gpt-5.2\"}}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"created\":1,\"model\":\"gpt-5.2\",\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3,\"total_tokens\":5}}}\n\ndata: [DONE]\n";
        let out = sse_chat_completion_to_json(sse).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["id"], "resp_123");
        assert_eq!(v["choices"][0]["message"]["content"], "helloworld");
        assert_eq!(v["usage"]["total_tokens"], 5);
    }

    #[test]
    fn chat_chunk_content_array_is_aggregated() {
        let sse = b"data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello\"}]},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"gpt\",\"choices\":[{\"index\":0,\"delta\":{\"content\":[{\"type\":\"output_text\",\"text\":\"world\"}]},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n";
        let out = sse_chat_completion_to_json(sse).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "helloworld");
    }

    #[test]
    fn openai_error_payload_is_treated_as_error() {
        let raw = serde_json::json!({
            "error": {
                "type": "invalid_api_key",
                "message": "bad key"
            }
        });
        let err = gateway_error_from_openai_payload(&raw).unwrap();
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn fallback_response_extracts_array_content() {
        let raw = serde_json::json!({
            "id": "chatcmpl-array",
            "object": "chat.completion",
            "created": 1,
            "model": "gpt-5.2",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "hello"},
                        {"type": "output_text", "text": "world"}
                    ]
                },
                "finish_reason": "stop"
            }]
        });
        let typed = fallback_response_from_bytes(serde_json::to_string(&raw).unwrap().as_bytes()).unwrap();
        assert_eq!(typed.choices[0].message.content.as_deref(), Some("hello\n\nworld"));
    }
}
