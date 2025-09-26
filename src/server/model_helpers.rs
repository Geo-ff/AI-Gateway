use crate::config::ProviderType;
use crate::providers::openai::{ModelListResponse, Model, OpenAIProvider};

// 获取指定 Provider 的模型列表，并在需要时通过自定义端点获取
pub async fn fetch_provider_models(
    provider: &crate::config::Provider,
    api_key: &str,
) -> Result<Vec<Model>, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(models_endpoint) = &provider.models_endpoint {
        let full_url = format!("{}{}", provider.base_url.trim_end_matches('/'), models_endpoint);
        let response = fetch_models_from_endpoint(&full_url, api_key).await?;
        Ok(response.data)
    } else {
        match provider.api_type {
            ProviderType::OpenAI => {
                let response = OpenAIProvider::list_models(&provider.base_url, api_key).await?;
                Ok(response.data)
            }
            ProviderType::Anthropic => {
                use std::io::{Error, ErrorKind};
                Err(Error::new(ErrorKind::Other, "Anthropic models listing not implemented").into())
            }
        }
    }
}

// 从指定 URL 获取模型列表（OpenAI 兼容响应）
async fn fetch_models_from_endpoint(
    url: &str,
    api_key: &str,
) -> Result<ModelListResponse, reqwest::Error> {
    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .send()
        .await?;

    response.json::<ModelListResponse>().await
}

