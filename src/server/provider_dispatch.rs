use crate::config::ProviderType;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, OpenAIProvider};
use crate::routing::{LoadBalancer, SelectedProvider, load_balancer::BalanceError};
use crate::server::AppState;
use crate::server::model_parser::ParsedModel;

// 基于请求的模型名称选择合适的供应商
pub fn select_provider_for_model(
    app_state: &AppState,
    model_name: &str,
) -> Result<(SelectedProvider, ParsedModel), BalanceError> {
    let parsed_model = ParsedModel::parse(model_name);

    // 如果解析出了供应商前缀，尝试直接匹配该供应商
    if let Some(provider_name) = &parsed_model.provider_name {
        if let Some(provider_config) = app_state.config.providers.get(provider_name) {
            if let Some(api_key) = provider_config.api_keys.first() {
                let selected = SelectedProvider {
                    provider: provider_config.clone(),
                    api_key: api_key.clone(),
                };
                return Ok((selected, parsed_model));
            }
        }
        // 如果指定的供应商不存在或没有可用的 API 密钥，返回错误
        return Err(BalanceError::NoProvidersAvailable);
    }

    // 没有指定供应商前缀，使用负载均衡策略选择
    let selected = select_provider(app_state)?;
    Ok((selected, parsed_model))
}

// 基于当前配置选择一个可用的供应商（保留原有逻辑）
pub fn select_provider(app_state: &AppState) -> Result<SelectedProvider, BalanceError> {
    let providers: Vec<_> = app_state.config.providers.values().cloned().collect();
    let load_balancer = LoadBalancer::new(providers, app_state.config.load_balancing.strategy.clone());
    load_balancer.select_provider()
}

// 根据选中的供应商和解析的模型调用对应的聊天补全接口
pub async fn call_provider_with_parsed_model(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
    parsed_model: &ParsedModel,
) -> Result<ChatCompletionResponse, reqwest::Error> {
    // 创建一个新的请求，使用实际的模型名称
    let mut modified_request = request.clone();
    modified_request.model = parsed_model.get_upstream_model_name().to_string();

    match selected.provider.api_type {
        ProviderType::OpenAI => call_openai_provider(selected, &modified_request).await,
        ProviderType::Anthropic => call_anthropic_provider(selected, &modified_request).await,
    }
}

// 根据选中的供应商调用对应的聊天补全接口（保留原有逻辑）
pub async fn call_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<ChatCompletionResponse, reqwest::Error> {
    match selected.provider.api_type {
        ProviderType::OpenAI => call_openai_provider(selected, request).await,
        ProviderType::Anthropic => call_anthropic_provider(selected, request).await,
    }
}

async fn call_openai_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<ChatCompletionResponse, reqwest::Error> {
    OpenAIProvider::chat_completions(
        &selected.provider.base_url,
        &selected.api_key,
        request,
    )
    .await
}

async fn call_anthropic_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<ChatCompletionResponse, reqwest::Error> {
    let anthropic_request = AnthropicProvider::convert_openai_to_anthropic(request);

    let anthropic_response = AnthropicProvider::chat_completions(
        &selected.provider.base_url,
        &selected.api_key,
        &anthropic_request,
    )
    .await?;

    Ok(AnthropicProvider::convert_anthropic_to_openai(&anthropic_response))
}

