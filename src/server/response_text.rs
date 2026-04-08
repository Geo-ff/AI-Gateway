use crate::error::GatewayError;
use crate::providers::openai::types::{ChatCompletionResponse, RawAndTypedChatCompletion};
use serde_json::Value;

fn truncate(text: String, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text;
    }
    text.chars().take(max_len).collect::<String>() + "…"
}

fn normalize_whitespace(text: String) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn collect_text_fragments(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => normalize_whitespace(text.clone()).into_iter().collect(),
        Value::Array(items) => items
            .iter()
            .flat_map(collect_text_fragments)
            .collect::<Vec<_>>(),
        Value::Object(map) => {
            for key in ["output_text", "text", "value", "content"] {
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

fn join_fragments(fragments: Vec<String>) -> Option<String> {
    if fragments.is_empty() {
        return None;
    }
    let joined = fragments.join("\n\n");
    normalize_whitespace(joined)
}

fn collect_stream_fragments(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![text.clone()]
            }
        }
        Value::Array(items) => items
            .iter()
            .flat_map(collect_stream_fragments)
            .collect::<Vec<_>>(),
        Value::Object(map) => {
            for key in ["output_text", "text", "value", "content"] {
                if let Some(found) = map.get(key) {
                    let fragments = collect_stream_fragments(found);
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

fn join_stream_fragments(fragments: Vec<String>) -> Option<String> {
    if fragments.is_empty() {
        None
    } else {
        Some(fragments.concat())
    }
}

fn extract_from_choices(raw: &Value) -> Option<String> {
    let message = raw
        .get("choices")
        .and_then(|value| value.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"));

    if let Some(content) = message.and_then(|message| message.get("content"))
        && let Some(text) = join_fragments(collect_text_fragments(content))
    {
        return Some(text);
    }

    if let Some(text) = raw
        .get("choices")
        .and_then(|value| value.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("text"))
        .and_then(|value| join_fragments(collect_text_fragments(value)))
    {
        return Some(text);
    }

    None
}

fn extract_from_output(raw: &Value) -> Option<String> {
    if let Some(text) = raw
        .get("output_text")
        .and_then(|value| join_fragments(collect_text_fragments(value)))
    {
        return Some(text);
    }

    if let Some(text) = raw
        .get("output")
        .and_then(|value| join_fragments(collect_text_fragments(value)))
    {
        return Some(text);
    }

    None
}

fn extract_from_top_level_message(raw: &Value) -> Option<String> {
    if let Some(text) = raw
        .get("message")
        .and_then(|value| value.get("content"))
        .and_then(|value| join_fragments(collect_text_fragments(value)))
    {
        return Some(text);
    }

    raw.get("content")
        .and_then(|value| join_fragments(collect_text_fragments(value)))
}

fn extract_from_typed(typed: &ChatCompletionResponse) -> Option<String> {
    typed
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .and_then(normalize_whitespace)
}

pub(crate) fn extract_response_text(raw: &Value, typed: &ChatCompletionResponse) -> Option<String> {
    extract_from_choices(raw)
        .or_else(|| extract_from_output(raw))
        .or_else(|| extract_from_top_level_message(raw))
        .or_else(|| extract_from_typed(typed))
}

pub(crate) fn response_summary(dual: &RawAndTypedChatCompletion, max_len: usize) -> Option<String> {
    extract_response_text(&dual.raw, &dual.typed)
        .or_else(|| {
            serde_json::to_string(&dual.raw)
                .ok()
                .and_then(normalize_whitespace)
        })
        .map(|text| truncate(text, max_len))
}

pub(crate) fn response_preview(
    response: &Result<RawAndTypedChatCompletion, GatewayError>,
    success_max_len: usize,
    error_max_len: usize,
) -> Option<String> {
    match response {
        Ok(dual) => response_summary(dual, success_max_len),
        Err(err) => Some(truncate(err.to_string(), error_max_len)),
    }
}

pub(crate) fn preview_from_stream_text(text: String, max_len: usize) -> Option<String> {
    normalize_whitespace(text).map(|text| truncate(text, max_len))
}

pub(crate) fn stream_chunk_preview_fragment(raw: &Value) -> Option<String> {
    let delta = raw
        .get("choices")
        .and_then(|value| value.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"));

    if let Some(text) = delta
        .and_then(|delta| delta.get("content"))
        .and_then(|value| join_stream_fragments(collect_stream_fragments(value)))
    {
        return Some(text);
    }

    if let Some(text) = delta
        .and_then(|delta| delta.get("reasoning_content"))
        .and_then(|value| join_stream_fragments(collect_stream_fragments(value)))
    {
        return Some(text);
    }

    raw.get("choices")
        .and_then(|value| value.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("text"))
        .and_then(|value| join_stream_fragments(collect_stream_fragments(value)))
}

#[cfg(test)]
mod tests {
    use super::{extract_response_text, response_summary, stream_chunk_preview_fragment};
    use crate::providers::openai::types::RawAndTypedChatCompletion;
    use serde_json::json;

    fn dual_from_raw(raw: serde_json::Value) -> RawAndTypedChatCompletion {
        let typed = serde_json::from_value(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 0,
            "model": raw.get("model").cloned().unwrap_or_else(|| json!("test-model")),
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "typed fallback"},
                "finish_reason": "stop"
            }],
            "usage": raw.get("usage").cloned().unwrap_or_else(|| json!(null))
        }))
        .unwrap();
        RawAndTypedChatCompletion { typed, raw }
    }

    #[test]
    fn extracts_text_from_standard_chat_choice() {
        let dual = dual_from_raw(json!({
            "id": "chatcmpl-standard",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }]
        }));

        assert_eq!(
            extract_response_text(&dual.raw, &dual.typed).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn extracts_text_from_content_parts_array() {
        let dual = dual_from_raw(json!({
            "id": "chatcmpl-parts",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
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
        }));

        assert_eq!(
            extract_response_text(&dual.raw, &dual.typed).as_deref(),
            Some("hello\n\nworld")
        );
    }

    #[test]
    fn extracts_text_from_output_shape() {
        let dual = dual_from_raw(json!({
            "id": "resp-output",
            "object": "response",
            "created": 0,
            "model": "gpt-5.4",
            "output": [
                {
                    "type": "message",
                    "content": [
                        {"type": "output_text", "text": "hello from output"}
                    ]
                }
            ],
            "usage": {"prompt_tokens": 10, "completion_tokens": 12, "total_tokens": 22}
        }));

        assert_eq!(
            extract_response_text(&dual.raw, &dual.typed).as_deref(),
            Some("hello from output")
        );
    }

    #[test]
    fn response_summary_falls_back_to_json_when_text_missing() {
        let raw = json!({
            "id": "resp-json",
            "object": "response",
            "created": 0,
            "model": "test-model",
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        });
        let typed = serde_json::from_value(json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": null},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        }))
        .unwrap();
        let dual = RawAndTypedChatCompletion { typed, raw };

        let summary = response_summary(&dual, 1200).unwrap();
        assert!(summary.contains("response"));
        assert!(summary.contains("test-model"));
    }

    #[test]
    fn extracts_stream_chunk_preview_fragment_without_losing_spacing() {
        let chunk = json!({
            "id": "chatcmpl-stream",
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": {
                    "content": " hello"
                },
                "finish_reason": null
            }]
        });

        assert_eq!(
            stream_chunk_preview_fragment(&chunk).as_deref(),
            Some(" hello")
        );
    }

    #[test]
    fn extracts_stream_chunk_preview_fragment_from_reasoning_content() {
        let chunk = json!({
            "id": "chatcmpl-stream",
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": {
                    "reasoning_content": "step-1"
                },
                "finish_reason": null
            }]
        });

        assert_eq!(
            stream_chunk_preview_fragment(&chunk).as_deref(),
            Some("step-1")
        );
    }
}
