use crate::config::ProviderType;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, OpenAIProvider};
use crate::routing::{LoadBalancer, SelectedProvider, load_balancer::BalanceError};
use crate::server::AppState;

// 基于当前配置选择一个可用的供应商
pub fn select_provider(app_state: &AppState) -> Result<SelectedProvider, BalanceError> {
    let providers: Vec<_> = app_state.config.providers.values().cloned().collect();
    let load_balancer = LoadBalancer::new(providers, app_state.config.load_balancing.strategy.clone());
    load_balancer.select_provider()
}

// 根据选中的供应商调用对应的聊天补全接口
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

