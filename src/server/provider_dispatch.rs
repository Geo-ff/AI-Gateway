use crate::config::ProviderType;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, OpenAIProvider};
use crate::routing::{LoadBalancer, SelectedProvider, load_balancer::BalanceError};
use crate::server::AppState;
use crate::server::model_parser::ParsedModel;
use crate::error::GatewayError;

// 基于请求的模型名称选择合适的供应商
pub async fn select_provider_for_model(
    app_state: &AppState,
    model_name: &str,
) -> Result<(SelectedProvider, ParsedModel), BalanceError> {
    let parsed_model = ParsedModel::parse(model_name);

    // 如果解析出了供应商前缀，尝试直接匹配该供应商
    if let Some(provider_name) = &parsed_model.provider_name {
        if let Some(provider_config) = app_state.config.providers.get(provider_name) {
            // 优先从数据库获取动态密钥
            let db_keys = app_state
                .db
                .get_provider_keys(provider_name, &app_state.config.logging.key_log_strategy)
                .await
                .unwrap_or_default();
            if let Some(api_key) = db_keys.first().cloned().or_else(|| provider_config.api_keys.first().cloned()) {
                let selected = SelectedProvider { provider: provider_config.clone(), api_key };
                return Ok((selected, parsed_model));
            } else {
                return Err(BalanceError::NoApiKeysAvailable);
            }
        }
        // 指定供应商不存在
        return Err(BalanceError::NoProvidersAvailable);
    }

    // 没有指定供应商前缀，使用负载均衡策略选择
    let selected = select_provider(app_state).await?;
    Ok((selected, parsed_model))
}

// 基于当前配置选择一个可用的供应商（保留原有逻辑）
pub async fn select_provider(app_state: &AppState) -> Result<SelectedProvider, BalanceError> {
    let providers: Vec<_> = app_state.config.providers.values().cloned().collect();
    let load_balancer = LoadBalancer::new(providers, app_state.config.load_balancing.strategy.clone());
    let mut selected = load_balancer.select_provider()?;
    // 覆盖密钥为数据库动态密钥（若存在）
    if let Some(db_keys) = app_state
        .db
        .get_provider_keys(&selected.provider.name, &app_state.config.logging.key_log_strategy)
        .await
        .ok()
    {
        if let Some(first) = db_keys.first() {
            selected.api_key = first.clone();
        }
    }
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
