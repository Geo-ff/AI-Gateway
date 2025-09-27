use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse};
use async_openai::types as oai;
use anthropic_ai_sdk::types::message as anthropic;

pub struct AnthropicProvider;

impl AnthropicProvider {
    pub fn convert_openai_to_anthropic(openai_req: &ChatCompletionRequest) -> anthropic::CreateMessageParams {
        let system_prompt = extract_system_prompt(openai_req);

        let tools: Option<Vec<anthropic::Tool>> = openai_req.tools.as_ref().map(|tools| {
            tools.iter().map(|t| anthropic::Tool {
                name: t.function.name.clone(),
                description: t.function.description.clone(),
                input_schema: t.function.parameters.clone().unwrap_or_default(),
            }).collect()
        });

        let tool_choice = match openai_req.tool_choice.clone() {
            Some(oai::ChatCompletionToolChoiceOption::Named(named)) => Some(anthropic::ToolChoice::Tool { name: named.function.name }),
            Some(oai::ChatCompletionToolChoiceOption::Auto) => Some(anthropic::ToolChoice::Auto),
            Some(oai::ChatCompletionToolChoiceOption::Required) => Some(anthropic::ToolChoice::Any),
            Some(oai::ChatCompletionToolChoiceOption::None) => Some(anthropic::ToolChoice::None),
            None => None,
        };

        let mut mapped_messages: Vec<anthropic::Message> = Vec::with_capacity(openai_req.messages.len());
        for msg in &openai_req.messages {
            match msg {
                oai::ChatCompletionRequestMessage::Developer(_) | oai::ChatCompletionRequestMessage::System(_) => {
                    // handled via system prompt
                }
                oai::ChatCompletionRequestMessage::User(m) => {
                    let content = match &m.content {
                        oai::ChatCompletionRequestUserMessageContent::Text(text) => anthropic::MessageContent::Text { content: text.clone() },
                        oai::ChatCompletionRequestUserMessageContent::Array(parts) => {
                            let blocks = parts.iter().filter_map(|p| match p {
                                oai::ChatCompletionRequestUserMessageContentPart::Text(t) => Some(anthropic::ContentBlock::Text { text: t.text.clone() }),
                                oai::ChatCompletionRequestUserMessageContentPart::ImageUrl(img) => {
                                    let (src_type, media_type, data_or_url) = image_source_from_url(&img.image_url.url);
                                    Some(anthropic::ContentBlock::Image { source: anthropic::ImageSource { type_: src_type, media_type, data: data_or_url } })
                                }
                                oai::ChatCompletionRequestUserMessageContentPart::InputAudio(_) => None,
                            }).collect();
                            anthropic::MessageContent::Blocks { content: blocks }
                        }
                    };
                    mapped_messages.push(anthropic::Message { role: anthropic::Role::User, content });
                }
                oai::ChatCompletionRequestMessage::Assistant(m) => {
                    let mut blocks: Vec<anthropic::ContentBlock> = Vec::new();
                    if let Some(content) = &m.content {
                        match content {
                            oai::ChatCompletionRequestAssistantMessageContent::Text(text) => if !text.is_empty() { blocks.push(anthropic::ContentBlock::Text { text: text.clone() }); },
                            oai::ChatCompletionRequestAssistantMessageContent::Array(parts) => {
                                for p in parts {
                                    match p {
                                        oai::ChatCompletionRequestAssistantMessageContentPart::Text(t) => blocks.push(anthropic::ContentBlock::Text { text: t.text.clone() }),
                                        oai::ChatCompletionRequestAssistantMessageContentPart::Refusal(r) => blocks.push(anthropic::ContentBlock::Text { text: r.refusal.clone() }),
                                    }
                                }
                            }
                        }
                    }
                    if let Some(tool_calls) = &m.tool_calls {
                        for tc in tool_calls {
                            let input = if tc.function.arguments.is_empty() {
                                serde_json::json!({})
                            } else {
                                serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| serde_json::json!({}))
                            };
                            blocks.push(anthropic::ContentBlock::ToolUse { id: tc.id.clone(), name: tc.function.name.clone(), input });
                        }
                    }
                    if !blocks.is_empty() {
                        mapped_messages.push(anthropic::Message { role: anthropic::Role::Assistant, content: anthropic::MessageContent::Blocks { content: blocks } });
                    }
                }
                oai::ChatCompletionRequestMessage::Tool(m) => {
                    let blocks = match &m.content {
                        oai::ChatCompletionRequestToolMessageContent::Text(text) => vec![anthropic::ContentBlock::ToolResult { tool_use_id: m.tool_call_id.clone(), content: text.clone() }],
                        oai::ChatCompletionRequestToolMessageContent::Array(parts) => parts.iter().map(|p| match p {
                            oai::ChatCompletionRequestToolMessageContentPart::Text(t) => anthropic::ContentBlock::ToolResult { tool_use_id: m.tool_call_id.clone(), content: t.text.clone() },
                        }).collect(),
                    };
                    mapped_messages.push(anthropic::Message { role: anthropic::Role::User, content: anthropic::MessageContent::Blocks { content: blocks } });
                }
                oai::ChatCompletionRequestMessage::Function(m) => {
                    if let Some(tool) = tools.as_ref().and_then(|ts| ts.iter().find(|t| t.name == m.name)) {
                        mapped_messages.push(anthropic::Message { role: anthropic::Role::Assistant, content: anthropic::MessageContent::Blocks { content: vec![anthropic::ContentBlock::ToolUse { id: m.name.clone(), name: tool.name.clone(), input: tool.input_schema.clone() }] } });
                    }
                }
            }
        }

        let stop_sequences = match openai_req.stop.clone() {
            Some(oai::Stop::String(s)) => Some(vec![s]),
            Some(oai::Stop::StringArray(v)) => Some(v),
            None => None,
        };
        let max_tokens = openai_req.max_completion_tokens.or(openai_req.max_tokens).unwrap_or(1024);

        anthropic::CreateMessageParams {
            max_tokens,
            messages: mapped_messages,
            model: openai_req.model.clone(),
            system: system_prompt,
            temperature: openai_req.temperature,
            stop_sequences,
            stream: openai_req.stream,
            top_k: None,
            top_p: openai_req.top_p,
            tools,
            tool_choice,
            metadata: openai_req.user.as_ref().map(|u| anthropic::Metadata { fields: std::collections::HashMap::from([(String::from("user_id"), u.clone())]) }),
            thinking: None,
        }
    }

    pub fn convert_anthropic_to_openai(resp: &anthropic::CreateMessageResponse) -> ChatCompletionResponse {
        use async_openai::types as openai;
        let mut text = String::new();
        let mut tool_calls: Vec<openai::ChatCompletionMessageToolCall> = Vec::new();
        for block in &resp.content {
            match block {
                anthropic::ContentBlock::Text { text: t } => {
                    if !text.is_empty() { text.push('\n'); }
                    text.push_str(t);
                }
                anthropic::ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(openai::ChatCompletionMessageToolCall {
                        id: id.clone(),
                        r#type: openai::ChatCompletionToolType::Function,
                        function: openai::FunctionCall { name: name.clone(), arguments: serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()) },
                    });
                }
                anthropic::ContentBlock::ToolResult { tool_use_id: _, content: c } => {
                    if !text.is_empty() { text.push('\n'); }
                    text.push_str(c);
                }
                anthropic::ContentBlock::Image { .. } => {}
                anthropic::ContentBlock::Thinking { .. } => {}
                anthropic::ContentBlock::RedactedThinking { .. } => {}
            }
        }

        let role = match resp.role { anthropic::Role::User => oai::Role::User, anthropic::Role::Assistant => oai::Role::Assistant };
        let finish_reason = match resp.stop_reason {
            Some(anthropic::StopReason::EndTurn) | Some(anthropic::StopReason::StopSequence) => Some(oai::FinishReason::Stop),
            Some(anthropic::StopReason::MaxTokens) => Some(oai::FinishReason::Length),
            Some(anthropic::StopReason::ToolUse) => Some(oai::FinishReason::ToolCalls),
            Some(anthropic::StopReason::Refusal) => Some(oai::FinishReason::ContentFilter),
            None => None,
        };

        let usage = oai::CompletionUsage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        };

        let message = oai::ChatCompletionResponseMessage {
            role,
            content: if text.is_empty() { None } else { Some(text) },
            refusal: None,
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
            function_call: None,
            audio: None,
        };

        oai::CreateChatCompletionResponse {
            id: resp.id.clone(),
            object: "chat.completion".to_string(),
            created: chrono::Utc::now().timestamp() as u32,
            model: resp.model.clone(),
            choices: vec![oai::ChatChoice { index: 0, message, finish_reason, logprobs: None }],
            usage: Some(usage),
            service_tier: None,
            system_fingerprint: None,
        }
    }

    pub async fn chat_completions(
        base_url: &str,
        api_key: &str,
        request: &anthropic::CreateMessageParams,
    ) -> crate::error::Result<anthropic::CreateMessageResponse> {
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
        Ok(response.json::<anthropic::CreateMessageResponse>().await?)
    }
}

fn image_source_from_url(url: &str) -> (String, String, String) {
    if url.starts_with("http://") || url.starts_with("https://") {
        ("url".to_string(), String::new(), url.to_string())
    } else if let Some(rest) = url.strip_prefix("data:") {
        // format: data:<mime>;base64,<data>
        let mut parts = rest.splitn(2, ',');
        let meta = parts.next().unwrap_or("");
        let data = parts.next().unwrap_or("");
        let mime = meta.split(';').next().unwrap_or("application/octet-stream");
        ("base64".to_string(), mime.to_string(), data.to_string())
    } else {
        ("url".to_string(), String::new(), url.to_string())
    }
}

fn extract_system_prompt(openai_req: &ChatCompletionRequest) -> Option<String> {
    for msg in &openai_req.messages {
        match msg {
            oai::ChatCompletionRequestMessage::Developer(dev) => {
                return match &dev.content {
                    oai::ChatCompletionRequestDeveloperMessageContent::Text(s) => Some(s.clone()),
                    oai::ChatCompletionRequestDeveloperMessageContent::Array(parts) => Some(parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")),
                }
            }
            oai::ChatCompletionRequestMessage::System(sys) => {
                return match &sys.content {
                    oai::ChatCompletionRequestSystemMessageContent::Text(s) => Some(s.clone()),
                    oai::ChatCompletionRequestSystemMessageContent::Array(parts) => Some(parts.iter().map(|p| match p { oai::ChatCompletionRequestSystemMessageContentPart::Text(t) => t.text.as_str() }).collect::<Vec<_>>().join("\n")),
                }
            }
            _ => {}
        }
    }
    None
}

// duplicate removed
