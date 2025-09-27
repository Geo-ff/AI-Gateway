use crate::config::ProviderType;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, OpenAIProvider};
use crate::providers::zhipu as zhipu;
use crate::routing::{LoadBalancer, SelectedProvider, load_balancer::BalanceError};
use crate::server::AppState;
use crate::server::model_parser::ParsedModel;
use crate::error::GatewayError;

// 基于请求的模型名称选择合适的供应商
pub async fn select_provider_for_model(
    app_state: &AppState,
    model_name: &str,
) -> Result<(SelectedProvider, ParsedModel), GatewayError> {
    let parsed_model = ParsedModel::parse(model_name);

    // 如果解析出了供应商前缀，尝试直接匹配该供应商（从数据库读取）
    if let Some(provider_name) = &parsed_model.provider_name {
        if let Some(provider) = app_state
            .db
            .get_provider(provider_name)
            .await
            .ok()
            .flatten()
        {
            let db_keys = app_state
                .db
                .get_provider_keys(provider_name, &app_state.config.logging.key_log_strategy)
                .await
                .unwrap_or_default();
            if let Some(api_key) = db_keys.first().cloned() {
                let selected = SelectedProvider { provider, api_key };
                return Ok((selected, parsed_model));
            } else {
                return Err(GatewayError::from(BalanceError::NoApiKeysAvailable));
            }
        } else {
            // 指定供应商不存在
            return Err(GatewayError::NotFound(format!("Provider '{}' not found", provider_name)));
        }
    }

    // 没有指定供应商前缀，使用负载均衡策略选择
    let selected = select_provider(app_state).await.map_err(GatewayError::from)?;
    Ok((selected, parsed_model))
}

// 基于数据库中可用的供应商进行选择（替代文件配置）
pub async fn select_provider(app_state: &AppState) -> Result<SelectedProvider, BalanceError> {
    // 从数据库读取所有供应商，并为其填充动态密钥
    let mut providers = app_state
        .db
        .list_providers_with_keys(&app_state.config.logging.key_log_strategy)
        .await
        .map_err(|_| BalanceError::NoProvidersAvailable)?;

    // 仅保留至少一个可用密钥的供应商
    providers.retain(|p| !p.api_keys.is_empty());
    let load_balancer = LoadBalancer::new(providers, app_state.config.load_balancing.strategy.clone());
    let selected = load_balancer.select_provider()?;
    Ok(selected)
}

// 根据选中的供应商和解析的模型调用对应的聊天补全接口
pub async fn call_provider_with_parsed_model(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
    parsed_model: &ParsedModel,
) -> Result<ChatCompletionResponse, GatewayError> {
    // 创建一个新的请求，使用实际的模型名称
    let mut modified_request = request.clone();
    modified_request.model = parsed_model.get_upstream_model_name().to_string();

    match selected.provider.api_type {
        ProviderType::OpenAI => call_openai_provider(selected, &modified_request).await,
        ProviderType::Anthropic => call_anthropic_provider(selected, &modified_request).await,
        ProviderType::Zhipu => call_zhipu_provider(selected, &modified_request).await,
    }
}

// 根据选中的供应商调用对应的聊天补全接口（保留原有逻辑）
// 旧的通用调用函数已移除（不再使用）

async fn call_openai_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<ChatCompletionResponse, GatewayError> {
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
) -> Result<ChatCompletionResponse, GatewayError> {
    let anthropic_request = AnthropicProvider::convert_openai_to_anthropic(request);

    let anthropic_response = AnthropicProvider::chat_completions(
        &selected.provider.base_url,
        &selected.api_key,
        &anthropic_request,
    )
    .await
    .map_err(GatewayError::from)?;

    Ok(AnthropicProvider::convert_anthropic_to_openai(&anthropic_response))
}

async fn call_zhipu_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<ChatCompletionResponse, GatewayError> {
    let adapted = zhipu::adapt_openai_request_for_zhipu(request.clone());
    let resp = zhipu::chat_completions(
        &selected.provider.base_url,
        &selected.api_key,
        &adapted,
    )
    .await?;
    Ok(resp)
}
