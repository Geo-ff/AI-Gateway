use crate::config::ProviderType;
use crate::error::{GatewayError, Result as AppResult};
use crate::providers::openai::{Model, ModelListResponse, OpenAIProvider};
use crate::providers::{adapters::ListModelsRequest, adapters::adapter_for};

// 获取指定 Provider 的模型列表，并在需要时通过自定义端点获取
pub async fn fetch_provider_models(
    provider: &crate::config::Provider,
    api_key: &str,
) -> AppResult<Vec<Model>> {
    if let Some(models_endpoint) = &provider.models_endpoint {
        let full_url = format!(
            "{}{}",
            provider.base_url.trim_end_matches('/'),
            models_endpoint
        );
        let response = fetch_models_from_endpoint(&full_url, api_key, provider.api_type).await?;
        Ok(response.data)
    } else {
        if matches!(
            provider.api_type,
            ProviderType::OpenAI | ProviderType::Doubao
        ) {
            let response = OpenAIProvider::list_models(&provider.base_url, api_key).await?;
            Ok(response.data)
        } else if !provider
            .api_type
            .capabilities()
            .supports_auto_model_discovery
        {
            Err(GatewayError::Config(
                format!(
                    "provider type '{}' does not support direct models listing yet",
                    provider.api_type.as_str()
                )
                .into(),
            ))
        } else {
            let base_url = reqwest::Url::parse(&provider.base_url)
                .map_err(|e| GatewayError::Config(e.to_string().into()))?;
            let models_url = crate::server::ssrf::join_models_url(&base_url, None)?;
            let adapter = adapter_for(provider.api_type).ok_or_else(|| {
                GatewayError::Config(
                    format!(
                        "provider type '{}' does not have a models adapter",
                        provider.api_type.as_str()
                    )
                    .into(),
                )
            })?;
            let data = adapter
                .list_models(ListModelsRequest {
                    models_url: &models_url,
                    api_key,
                })
                .await?
                .into_iter()
                .map(|id| Model {
                    id,
                    object: "model".into(),
                    created: 0,
                    owned_by: provider.name.clone(),
                    display_name: None,
                })
                .collect();
            Ok(data)
        }
    }
}

// 从指定 URL 获取模型列表（OpenAI 兼容响应）
async fn fetch_models_from_endpoint(
    url: &str,
    api_key: &str,
    provider_type: ProviderType,
) -> Result<ModelListResponse, GatewayError> {
    let client = crate::http_client::client_for_url(url)?;
    let adapter = adapter_for(provider_type).ok_or_else(|| {
        GatewayError::Config(
            format!(
                "provider type '{}' does not have a models adapter",
                provider_type.as_str()
            )
            .into(),
        )
    })?;
    let mut request = client.get(url).header("Content-Type", "application/json");
    for (name, value) in adapter.build_auth_headers(api_key).map_err(|(_, detail)| {
        GatewayError::Config(
            detail
                .unwrap_or_else(|| "invalid auth header".into())
                .into(),
        )
    })? {
        if let Some(name) = name {
            request = request.header(name, value);
        }
    }
    let response = request.send().await?;

    Ok(response.json::<ModelListResponse>().await?)
}
