use anthropic_ai_sdk::types::message as anthropic;

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
