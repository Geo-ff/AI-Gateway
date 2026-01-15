use crate::config::ProviderType;
use crate::error::GatewayError;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::{ChatCompletionRequest, OpenAIProvider, RawAndTypedChatCompletion};
use crate::providers::zhipu;
use crate::routing::{LoadBalancer, SelectedProvider, load_balancer::BalanceError};
use crate::server::AppState;
use crate::server::model_parser::ParsedModel;

// 基于请求的模型名称选择合适的供应商
pub async fn select_provider_for_model(
    app_state: &AppState,
    model_name: &str,
) -> Result<(SelectedProvider, ParsedModel), GatewayError> {
    let parsed_model = ParsedModel::parse(model_name);

    // 如果解析出了供应商前缀，尝试直接匹配该供应商（从数据库读取）
    if let Some(provider_name) = &parsed_model.provider_name {
        if let Some(provider) = app_state
            .providers
            .get_provider(provider_name)
            .await
            .ok()
            .flatten()
        {
            if !provider.enabled {
                return Err(GatewayError::Forbidden(format!(
                    "Provider '{}' is disabled",
                    provider_name
                )));
            }
            let keys = app_state
                .providers
                .list_provider_keys_raw(provider_name, &app_state.config.logging.key_log_strategy)
                .await
                .unwrap_or_default();
            let strategy = app_state
                .providers
                .get_provider_key_rotation_strategy(provider_name)
                .await
                .unwrap_or_default();
            let api_key = app_state
                .load_balancer_state
                .select_provider_key(provider_name, strategy, &keys)?;
            if api_key.is_empty() {
                return Err(GatewayError::from(BalanceError::NoApiKeysAvailable));
            }
            return Ok((
                SelectedProvider {
                    provider,
                    api_key,
                },
                parsed_model,
            ));
        } else {
            // 指定供应商不存在
            return Err(GatewayError::NotFound(format!(
                "Provider '{}' not found",
                provider_name
            )));
        }
    }

    // 没有指定供应商前缀，使用负载均衡策略选择
    let selected = select_provider(app_state)
        .await
        .map_err(GatewayError::from)?;
    Ok((selected, parsed_model))
}

// 基于数据库中可用的供应商进行选择（替代文件配置）
pub async fn select_provider(app_state: &AppState) -> Result<SelectedProvider, BalanceError> {
    let providers = app_state
        .providers
        .list_providers()
        .await
        .map_err(|_| BalanceError::NoProvidersAvailable)?;

    if providers.is_empty() {
        return Err(BalanceError::NoProvidersAvailable);
    }

    let mut candidates: Vec<crate::config::Provider> = Vec::new();
    let mut keys_by_provider: std::collections::HashMap<String, Vec<crate::routing::ProviderKeyEntry>> =
        std::collections::HashMap::new();

    for p in providers {
        if !p.enabled {
            continue;
        }
        let keys = app_state
            .providers
            .list_provider_keys_raw(&p.name, &app_state.config.logging.key_log_strategy)
            .await
            .unwrap_or_default();
        let has_active = keys.iter().any(|k| k.active && !k.value.is_empty() && k.weight >= 1);
        if has_active {
            keys_by_provider.insert(p.name.clone(), keys);
            candidates.push(p);
        }
    }

    if candidates.is_empty() {
        return Err(BalanceError::NoApiKeysAvailable);
    }

    let load_balancer = LoadBalancer::with_state(
        candidates,
        app_state.config.load_balancing.strategy.clone(),
        app_state.load_balancer_state.clone(),
    );
    let provider = load_balancer.select_provider_only()?;

    let keys = keys_by_provider
        .remove(&provider.name)
        .unwrap_or_default();
    let strategy = app_state
        .providers
        .get_provider_key_rotation_strategy(&provider.name)
        .await
        .unwrap_or_default();
    let api_key = app_state
        .load_balancer_state
        .select_provider_key(&provider.name, strategy, &keys)?;

    Ok(SelectedProvider { provider, api_key })
}

// 根据选中的供应商和解析的模型调用对应的聊天补全接口
pub async fn call_provider_with_parsed_model(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
    parsed_model: &ParsedModel,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    // 创建一个新的请求，使用实际的模型名称
    let mut modified_request = request.clone();
    modified_request.model = parsed_model.get_upstream_model_name().to_string();

    match selected.provider.api_type {
        ProviderType::OpenAI => call_openai_provider(selected, &modified_request).await,
        ProviderType::Anthropic => call_anthropic_provider(selected, &modified_request).await,
        ProviderType::Zhipu => call_zhipu_provider(selected, &modified_request).await,
    }
}

async fn call_openai_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    OpenAIProvider::chat_completions(&selected.provider.base_url, &selected.api_key, request).await
}

async fn call_anthropic_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let anthropic_request = AnthropicProvider::convert_openai_to_anthropic(request);

    let anthropic_response = AnthropicProvider::chat_completions(
        &selected.provider.base_url,
        &selected.api_key,
        &anthropic_request,
    )
    .await?;

    Ok(RawAndTypedChatCompletion {
        typed: AnthropicProvider::convert_anthropic_to_openai(&anthropic_response),
        raw: serde_json::to_value(AnthropicProvider::convert_anthropic_to_openai(
            &anthropic_response,
        ))
        .unwrap_or(serde_json::json!({})),
    })
}

async fn call_zhipu_provider(
    selected: &SelectedProvider,
    request: &ChatCompletionRequest,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let adapted = zhipu::adapt_openai_request_for_zhipu(request.clone());
    let resp =
        zhipu::chat_completions(&selected.provider.base_url, &selected.api_key, &adapted).await?;
    Ok(resp)
}
