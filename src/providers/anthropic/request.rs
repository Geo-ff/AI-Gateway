use anthropic_ai_sdk::types::message as anthropic;
use async_openai::types as oai;

use crate::providers::openai::ChatCompletionRequest;

use super::utils::{extract_system_prompt, image_source_from_url};

pub fn convert_openai_to_anthropic(
    openai_req: &ChatCompletionRequest,
) -> anthropic::CreateMessageParams {
    let system_prompt = extract_system_prompt(openai_req);

    let tools: Option<Vec<anthropic::Tool>> = openai_req.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|t| anthropic::Tool {
                name: t.function.name.clone(),
                description: t.function.description.clone(),
                input_schema: t.function.parameters.clone().unwrap_or_default(),
            })
            .collect()
    });

    let tool_choice = match openai_req.tool_choice.clone() {
        Some(oai::ChatCompletionToolChoiceOption::Named(named)) => {
            Some(anthropic::ToolChoice::Tool {
                name: named.function.name,
            })
        }
        Some(oai::ChatCompletionToolChoiceOption::Auto) => Some(anthropic::ToolChoice::Auto),
        Some(oai::ChatCompletionToolChoiceOption::Required) => Some(anthropic::ToolChoice::Any),
        Some(oai::ChatCompletionToolChoiceOption::None) => Some(anthropic::ToolChoice::None),
        None => None,
    };

    let mut mapped_messages: Vec<anthropic::Message> =
        Vec::with_capacity(openai_req.messages.len());
    for msg in &openai_req.messages {
        match msg {
            oai::ChatCompletionRequestMessage::Developer(_)
            | oai::ChatCompletionRequestMessage::System(_) => {
                // handled via system prompt
            }
            oai::ChatCompletionRequestMessage::Function(_) => {
                // legacy function message: ignore; handled by tools/tool_calls
            }
            oai::ChatCompletionRequestMessage::User(m) => {
                let content = match &m.content {
                    oai::ChatCompletionRequestUserMessageContent::Text(text) => {
                        anthropic::MessageContent::Text {
                            content: text.clone(),
                        }
                    }
                    oai::ChatCompletionRequestUserMessageContent::Array(parts) => {
                        let blocks = parts
                            .iter()
                            .filter_map(|p| match p {
                                oai::ChatCompletionRequestUserMessageContentPart::Text(t) => {
                                    Some(anthropic::ContentBlock::Text {
                                        text: t.text.clone(),
                                    })
                                }
                                oai::ChatCompletionRequestUserMessageContentPart::ImageUrl(img) => {
                                    let (src_type, media_type, data_or_url) =
                                        image_source_from_url(&img.image_url.url);
                                    Some(anthropic::ContentBlock::Image {
                                        source: anthropic::ImageSource {
                                            type_: src_type,
                                            media_type,
                                            data: data_or_url,
                                        },
                                    })
                                }
                                oai::ChatCompletionRequestUserMessageContentPart::InputAudio(_) => {
                                    None
                                }
                            })
                            .collect();
                        anthropic::MessageContent::Blocks { content: blocks }
                    }
                };
                mapped_messages.push(anthropic::Message {
                    role: anthropic::Role::User,
                    content,
                });
            }
            oai::ChatCompletionRequestMessage::Assistant(m) => {
                let mut blocks: Vec<anthropic::ContentBlock> = Vec::new();
                if let Some(content) = &m.content {
                    match content {
                        oai::ChatCompletionRequestAssistantMessageContent::Text(text) => {
                            if !text.is_empty() {
                                blocks.push(anthropic::ContentBlock::Text { text: text.clone() })
                            }
                        }
                        oai::ChatCompletionRequestAssistantMessageContent::Array(parts) => {
                            for p in parts {
                                match p {
                                    oai::ChatCompletionRequestAssistantMessageContentPart::Text(t) => {
                                        blocks.push(anthropic::ContentBlock::Text { text: t.text.clone() })
                                    }
                                    oai::ChatCompletionRequestAssistantMessageContentPart::Refusal(r) => {
                                        blocks.push(anthropic::ContentBlock::Text { text: r.refusal.clone() })
                                    }
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
                            serde_json::from_str(&tc.function.arguments)
                                .unwrap_or_else(|_| serde_json::json!({}))
                        };
                        blocks.push(anthropic::ContentBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            input,
                        });
                    }
                }
                if !blocks.is_empty() {
                    mapped_messages.push(anthropic::Message {
                        role: anthropic::Role::Assistant,
                        content: anthropic::MessageContent::Blocks { content: blocks },
                    });
                }
            }
            oai::ChatCompletionRequestMessage::Tool(m) => {
                // OpenAI tool results -> Anthropic tool_result content block
                let mut blocks: Vec<anthropic::ContentBlock> = Vec::new();
                let id = m.tool_call_id.clone();
                let content_str = match &m.content {
                    oai::ChatCompletionRequestToolMessageContent::Text(t) => t.clone(),
                    oai::ChatCompletionRequestToolMessageContent::Array(parts) => parts
                        .iter()
                        .map(|p| match p {
                            oai::ChatCompletionRequestToolMessageContentPart::Text(t) => {
                                t.text.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                blocks.push(anthropic::ContentBlock::ToolResult {
                    tool_use_id: id,
                    content: content_str,
                });
                mapped_messages.push(anthropic::Message {
                    role: anthropic::Role::User,
                    content: anthropic::MessageContent::Blocks { content: blocks },
                });
            }
        }
    }

    anthropic::CreateMessageParams {
        model: openai_req.model.clone(),
        system: system_prompt,
        messages: mapped_messages,
        tools,
        tool_choice,
        max_tokens: openai_req
            .max_completion_tokens
            .or(openai_req.max_tokens)
            .unwrap_or(1024) as u32,
        temperature: Some(openai_req.temperature.unwrap_or(1.0) as f32),
        top_p: Some(openai_req.top_p.unwrap_or(1.0) as f32),
        stream: Some(openai_req.stream.unwrap_or(false)),
        ..Default::default()
    }
}
