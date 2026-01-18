use anthropic_ai_sdk::types::message as anthropic;
use async_openai::types as oai;

pub fn extract_reasoning_content(resp: &anthropic::CreateMessageResponse) -> Option<String> {
    let mut reasoning = String::new();
    let mut has_redacted = false;
    for block in &resp.content {
        match block {
            anthropic::ContentBlock::Thinking { thinking, .. } => {
                if !reasoning.is_empty() {
                    reasoning.push('\n');
                }
                reasoning.push_str(thinking);
            }
            anthropic::ContentBlock::RedactedThinking { .. } => {
                has_redacted = true;
            }
            _ => {}
        }
    }

    if !reasoning.is_empty() {
        return Some(reasoning);
    }
    if has_redacted {
        return Some("[redacted_thinking]".to_string());
    }
    None
}

#[allow(deprecated)]
pub fn convert_anthropic_to_openai(
    resp: &anthropic::CreateMessageResponse,
) -> oai::CreateChatCompletionResponse {
    use async_openai::types as openai;
    let mut text = String::new();
    let mut tool_calls: Vec<openai::ChatCompletionMessageToolCall> = Vec::new();
    for block in &resp.content {
        match block {
            anthropic::ContentBlock::Text { text: t } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
            anthropic::ContentBlock::ToolUse { id, name, input } => {
                tool_calls.push(openai::ChatCompletionMessageToolCall {
                    id: id.clone(),
                    r#type: openai::ChatCompletionToolType::Function,
                    function: openai::FunctionCall {
                        name: name.clone(),
                        arguments: serde_json::to_string(input)
                            .unwrap_or_else(|_| "{}".to_string()),
                    },
                });
            }
            anthropic::ContentBlock::ToolResult {
                tool_use_id: _,
                content: c,
            } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(c);
            }
            anthropic::ContentBlock::Image { .. } => {}
            anthropic::ContentBlock::Thinking { .. } => {}
            anthropic::ContentBlock::RedactedThinking { .. } => {}
        }
    }

    let role = match resp.role {
        anthropic::Role::User => oai::Role::User,
        anthropic::Role::Assistant => oai::Role::Assistant,
    };
    let finish_reason = match resp.stop_reason {
        Some(anthropic::StopReason::EndTurn) | Some(anthropic::StopReason::StopSequence) => {
            Some(oai::FinishReason::Stop)
        }
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
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        function_call: None,
        audio: None,
    };

    oai::CreateChatCompletionResponse {
        id: resp.id.clone(),
        object: "chat.completion".to_string(),
        created: chrono::Utc::now().timestamp() as u32,
        model: resp.model.clone(),
        choices: vec![oai::ChatChoice {
            index: 0,
            message,
            finish_reason,
            logprobs: None,
        }],
        usage: Some(usage),
        service_tier: None,
        system_fingerprint: None,
    }
}
