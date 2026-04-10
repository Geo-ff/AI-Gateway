use anthropic_ai_sdk::types::message as anthropic;
use serde_json::Value;

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
    let status = response.status();
    let body = response.bytes().await?;

    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|error| error.get("message").or_else(|| error.get("detail")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                let text = String::from_utf8_lossy(&body).trim().to_string();
                if text.is_empty() { None } else { Some(text) }
            })
            .unwrap_or_else(|| format!("Anthropic upstream returned {}", status));
        return Err(crate::error::GatewayError::Config(message));
    }

    let raw: Value = serde_json::from_slice(&body)?;
    if raw.get("object").and_then(Value::as_str) == Some("chat.completion")
        || (raw.get("choices").is_some() && raw.get("content").is_none())
    {
        return Err(crate::error::GatewayError::Config(
            "当前供应商返回的是 OpenAI Chat Completions 格式，不是 Anthropic Messages 格式；请把该供应商类型改成 OpenAI 兼容，而不是 Anthropic。".into(),
        ));
    }

    Ok(serde_json::from_value(raw)?)
}
