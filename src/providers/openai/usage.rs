use super::types::{ChatCompletionResponse, Usage};
use serde_json::Value;

fn read_u32(value: Option<&Value>) -> Option<u32> {
    value.and_then(|item| item.as_u64()).map(|item| item as u32)
}

fn is_meaningful(usage: &Usage) -> bool {
    usage.prompt_tokens > 0
        || usage.completion_tokens > 0
        || usage.total_tokens > 0
        || usage
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .is_some()
        || usage
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .is_some()
}

fn parse_usage_object(usage: &Value) -> Option<Usage> {
    use async_openai::types::{CompletionTokensDetails, PromptTokensDetails};

    let tokens = usage.get("tokens");
    let billed_units = usage.get("billed_units");
    let prompt_tokens = read_u32(usage.get("prompt_tokens"))
        .or_else(|| read_u32(usage.get("input_tokens")))
        .or_else(|| read_u32(tokens.and_then(|value| value.get("input_tokens"))))
        .or_else(|| read_u32(billed_units.and_then(|value| value.get("input_tokens"))))
        .unwrap_or(0);
    let completion_tokens = read_u32(usage.get("completion_tokens"))
        .or_else(|| read_u32(usage.get("output_tokens")))
        .or_else(|| read_u32(tokens.and_then(|value| value.get("output_tokens"))))
        .or_else(|| read_u32(billed_units.and_then(|value| value.get("output_tokens"))))
        .unwrap_or(0);
    let total_tokens = read_u32(usage.get("total_tokens"))
        .or_else(|| {
            read_u32(usage.get("input_tokens"))
                .zip(read_u32(usage.get("output_tokens")))
                .map(|(input, output)| input + output)
        })
        .or_else(|| {
            read_u32(tokens.and_then(|value| value.get("input_tokens")))
                .zip(read_u32(
                    tokens.and_then(|value| value.get("output_tokens")),
                ))
                .map(|(input, output)| input + output)
        })
        .or_else(|| {
            read_u32(billed_units.and_then(|value| value.get("input_tokens")))
                .zip(read_u32(
                    billed_units.and_then(|value| value.get("output_tokens")),
                ))
                .map(|(input, output)| input + output)
        })
        .unwrap_or(prompt_tokens + completion_tokens);

    let cached_tokens = read_u32(
        usage
            .get("prompt_tokens_details")
            .and_then(|details| details.get("cached_tokens")),
    );
    let reasoning_tokens = read_u32(
        usage
            .get("completion_tokens_details")
            .and_then(|details| details.get("reasoning_tokens")),
    );

    let out = Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: cached_tokens.map(|cached_tokens| PromptTokensDetails {
            cached_tokens: Some(cached_tokens),
            audio_tokens: None,
        }),
        completion_tokens_details: reasoning_tokens.map(|reasoning_tokens| {
            CompletionTokensDetails {
                reasoning_tokens: Some(reasoning_tokens),
                audio_tokens: None,
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }
        }),
    };

    is_meaningful(&out).then_some(out)
}

fn merge_usage(primary: Usage, fallback: Usage) -> Usage {
    let prompt_tokens = if primary.prompt_tokens > 0 || fallback.prompt_tokens == 0 {
        primary.prompt_tokens
    } else {
        fallback.prompt_tokens
    };
    let completion_tokens = if primary.completion_tokens > 0 || fallback.completion_tokens == 0 {
        primary.completion_tokens
    } else {
        fallback.completion_tokens
    };
    let total_tokens = if primary.total_tokens > 0 || fallback.total_tokens == 0 {
        primary.total_tokens
    } else if prompt_tokens > 0 || completion_tokens > 0 {
        prompt_tokens + completion_tokens
    } else {
        fallback.total_tokens
    };

    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: primary
            .prompt_tokens_details
            .or(fallback.prompt_tokens_details),
        completion_tokens_details: primary
            .completion_tokens_details
            .or(fallback.completion_tokens_details),
    }
}

pub fn usage_from_value(value: &Value) -> Option<Usage> {
    value
        .get("usage")
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("usage"))
        })
        .and_then(parse_usage_object)
}

pub fn resolved_usage(raw: &Value, typed: &ChatCompletionResponse) -> Option<Usage> {
    match (typed.usage.clone(), usage_from_value(raw)) {
        (Some(primary), Some(fallback)) => Some(merge_usage(primary, fallback)),
        (Some(primary), None) => Some(primary),
        (None, Some(fallback)) => Some(fallback),
        (None, None) => None,
    }
    .filter(is_meaningful)
}

#[cfg(test)]
mod tests {
    use super::{resolved_usage, usage_from_value};
    use crate::providers::openai::types::ChatCompletionResponse;
    use serde_json::json;

    #[test]
    fn usage_from_value_supports_input_output_tokens() {
        let raw = json!({
            "usage": {
                "input_tokens": 10,
                "output_tokens": 12
            }
        });

        let usage = usage_from_value(&raw).unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 12);
        assert_eq!(usage.total_tokens, 22);
    }

    #[test]
    fn resolved_usage_prefers_raw_when_typed_usage_is_empty() {
        let raw = json!({
            "id": "resp_123",
            "object": "response",
            "created": 0,
            "model": "gpt-5.4",
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "hello"
                }]
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 12
            }
        });
        let typed: ChatCompletionResponse = serde_json::from_value(json!({
            "id": "resp_123",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-5.4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": null},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0
            }
        }))
        .unwrap();

        let usage = resolved_usage(&raw, &typed).unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 12);
        assert_eq!(usage.total_tokens, 22);
    }
}
