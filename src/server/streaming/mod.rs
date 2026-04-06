use axum::http::HeaderMap;
use axum::{
    extract::{Json, State},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use std::sync::Arc;

// Reuse API key hint from shared server utilities
use crate::error::GatewayError;
use crate::providers::adapters::runtime_streaming_unsupported_message;
use crate::server::AppState;
use crate::server::chat_request::GatewayChatCompletionRequest;
use crate::server::model_redirect::{
    apply_model_redirects, apply_provider_model_redirects_to_parsed_model,
};
use crate::server::pricing::{missing_price_allowed_for_chat, resolve_model_pricing};
use crate::server::provider_dispatch::select_provider_for_model;

mod anthropic;
mod common;
mod native;
mod openai;
mod zhipu;

/// Chat Completions 流式入口：
/// - 仅接受 `stream=true` 的请求，否则直接报错
/// - 应用模型重定向后，根据模型选择具体 Provider，并校验令牌额度/过期/模型白名单
/// - 按 Provider 类型分发到对应的流式实现（OpenAI/Zhipu/原生协议族），并统一返回 SSE 响应
pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(gateway_req): Json<GatewayChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    let top_k = gateway_req.top_k;
    let mut request = gateway_req.request;
    if !request.stream.unwrap_or(false) {
        return Err(GatewayError::Config(
            "stream=false for streaming endpoint".into(),
        ));
    }

    let start_time = Utc::now();
    let requested_model = request.model.clone();
    apply_model_redirects(&mut request);
    let parsed_for_prefix = crate::server::model_parser::ParsedModel::parse(&request.model);
    if let Some(p) = parsed_for_prefix.provider_name.as_deref() {
        let mut parsed = parsed_for_prefix.clone();
        if let Some((from, to)) =
            apply_provider_model_redirects_to_parsed_model(&app_state, p, &mut parsed).await?
        {
            return Err(GatewayError::Config(format!(
                "model '{}' is redirected; use '{}' instead",
                from, to
            )));
        }
    }
    let (selected, mut parsed_model) =
        select_provider_for_model(&app_state, &request.model).await?;

    // 若该模型在 provider redirects 中作为 source，则不允许第三方直接调用（避免 source/target 重复可用）
    let mut parsed_for_redirect_check = parsed_model.clone();
    if let Some((from, to)) = apply_provider_model_redirects_to_parsed_model(
        &app_state,
        &selected.provider.name,
        &mut parsed_for_redirect_check,
    )
    .await?
    {
        let ge = GatewayError::Config(format!(
            "model '{}' is redirected; use '{}' instead",
            from, to
        ));
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(from),
            Some(selected.provider.name.clone()),
            None,
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let mut redirected_from_for_price: Option<String> = None;
    if let Some((from, to)) = apply_provider_model_redirects_to_parsed_model(
        &app_state,
        &selected.provider.name,
        &mut parsed_model,
    )
    .await?
    {
        tracing::info!(
            provider = %selected.provider.name,
            source_model = %from,
            target_model = %to,
            "已应用 provider 维度模型重定向"
        );
        redirected_from_for_price = Some(from.clone());
        request.model = if parsed_model.provider_name.is_some() {
            format!("{}/{}", selected.provider.name, parsed_model.model_name)
        } else {
            parsed_model.model_name.clone()
        };
    }

    // Build upstream request with real model id
    let mut upstream_req = request.clone();
    upstream_req.model = parsed_model.get_upstream_model_name().to_string();

    // Extract required gateway token from Authorization header
    let client_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let client_token_log_id = client_token
        .as_deref()
        .map(crate::admin::client_token_id_for_token);
    let token_str = match client_token.as_deref() {
        Some(tok) => tok,
        None => {
            let ge = GatewayError::Config("missing bearer token".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                None,
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

    let token_record = app_state.token_store.get_token(token_str).await?;
    let token = match token_record {
        Some(t) => t,
        None => {
            let ge = GatewayError::Config("invalid token".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

    if let Some(user_id) = token.user_id.as_deref() {
        let user = app_state.user_store.get_user(user_id).await?;
        let balance = user.as_ref().map(|u| u.balance).unwrap_or(0.0);
        if balance <= 0.0 {
            let _ = app_state
                .token_store
                .set_enabled_for_user(user_id, false)
                .await;
            let ge = GatewayError::Config("余额不足：密钥已失效；充值/订阅后需手动启用密钥".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    }

    if !token.enabled {
        if let Some(max_amount) = token.max_amount
            && let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&token.id)
                .await
            && spent >= max_amount
        {
            let ge = GatewayError::Config("token budget exceeded".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_STREAM,
                Some(upstream_req.model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let ge = GatewayError::Config("token disabled".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token_log_id.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if let Some(exp) = token.expires_at
        && chrono::Utc::now() > exp
    {
        return Err(GatewayError::Config("token expired".into()));
    }

    crate::server::token_model_limits::enforce_model_allowed_for_token(&token, &request.model)?;

    if let Some(max_tokens) = token.max_tokens
        && token.total_tokens_spent >= max_tokens
    {
        let ge = GatewayError::Config("token tokens exceeded".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token_log_id.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if let Some(max_amount) = token.max_amount
        && let Ok(spent) = app_state
            .log_store
            .sum_spent_amount_by_client_token(&token.id)
            .await
        && spent > max_amount
    {
        return Err(GatewayError::Config("token budget exceeded".into()));
    }

    let upstream_model_for_check = parsed_model.get_upstream_model_name().to_string();
    if let Ok(Some(false)) = app_state
        .log_store
        .get_model_enabled(&selected.provider.name, &upstream_model_for_check)
        .await
    {
        let ge = GatewayError::Config("model is disabled".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token_log_id.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }
    let resolved_pricing = resolve_model_pricing(
        &app_state,
        &selected.provider.name,
        &upstream_model_for_check,
        redirected_from_for_price.as_deref(),
    )
    .await?;
    if !resolved_pricing.price_found && !missing_price_allowed_for_chat(&app_state) {
        let ge = GatewayError::Config("model price not set".into());
        let code = ge.status_code().as_u16();
        crate::server::request_logging::log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/v1/chat/completions",
            crate::logging::types::REQ_TYPE_CHAT_STREAM,
            Some(upstream_req.model.clone()),
            Some(selected.provider.name.clone()),
            client_token_log_id.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }
    if !resolved_pricing.price_found {
        tracing::warn!(
            provider = %selected.provider.name,
            model = %upstream_model_for_check,
            pricing_mode = ?app_state.config.server.pricing_mode,
            "missing model price; continuing without billing amount"
        );
    }
    let billing_model = resolved_pricing.billing_model;

    if let Some(message) = runtime_streaming_unsupported_message(selected.provider.api_type) {
        return Err(GatewayError::Config(message));
    }

    let response = match selected.provider.api_type {
        crate::config::ProviderType::Anthropic => anthropic::stream_anthropic_chat(
            app_state.clone(),
            start_time,
            billing_model.clone(),
            requested_model.clone(),
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            client_token.clone(),
            upstream_req,
            top_k,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Zhipu => zhipu::stream_zhipu_chat(
            app_state.clone(),
            start_time,
            billing_model.clone(),
            requested_model.clone(),
            upstream_req.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            client_token.clone(),
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::BaiduErnie | crate::config::ProviderType::XfSpark => {
            native::stream_native_chat(
                app_state.clone(),
                start_time,
                billing_model.clone(),
                requested_model.clone(),
                upstream_req.model.clone(),
                selected.provider.api_type,
                selected.provider.base_url.clone(),
                selected.provider.name.clone(),
                selected.api_key.clone(),
                client_token.clone(),
                upstream_req,
                selected.provider.provider_config.clone(),
            )
            .await
            .map(IntoResponse::into_response)
        }
        provider_type if provider_type.capabilities().openai_compatible => {
            openai::stream_openai_chat(
                app_state.clone(),
                start_time,
                billing_model.clone(),
                requested_model.clone(),
                upstream_req.model.clone(),
                selected.provider.base_url.clone(),
                selected.provider.name.clone(),
                selected.api_key.clone(),
                client_token.clone(),
                upstream_req,
            )
            .await
            .map(IntoResponse::into_response)
        }
        crate::config::ProviderType::AzureOpenAI
        | crate::config::ProviderType::GoogleGemini
        | crate::config::ProviderType::Cohere
        | crate::config::ProviderType::AwsClaude
        | crate::config::ProviderType::VertexAI => native::stream_native_chat(
            app_state.clone(),
            start_time,
            billing_model.clone(),
            requested_model.clone(),
            upstream_req.model.clone(),
            selected.provider.api_type,
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            client_token.clone(),
            upstream_req,
            selected.provider.provider_config.clone(),
        )
        .await
        .map(IntoResponse::into_response),
        provider_type => Err(GatewayError::Config(
            runtime_streaming_unsupported_message(provider_type).unwrap_or_else(|| {
                format!(
                    "provider type '{}' is registered but streaming dispatch is not implemented yet",
                    provider_type.as_str()
                )
            }),
        )),
    };

    if let Some(tok) = client_token.as_deref()
        && let Some(t) = app_state.token_store.get_token(tok).await?
    {
        if let Some(max_amount) = t.max_amount
            && t.amount_spent > max_amount
        {
            let _ = app_state.token_store.set_enabled(tok, false).await;
        }
        if let Some(max_tokens) = t.max_tokens
            && t.total_tokens_spent > max_tokens
        {
            let _ = app_state.token_store.set_enabled(tok, false).await;
        }
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::config::settings::{
        BalanceStrategy, LoadBalancing, LoggingConfig, PricingMode, Provider, ProviderConfig,
        ProviderType, ServerConfig,
    };
    use crate::logging::DatabaseLogger;
    use crate::providers::openai::ChatCompletionRequest;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use axum::Json;
    use axum::body::to_bytes;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Router, extract::State};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::net::TcpListener;

    fn test_settings(db_path: String) -> crate::config::Settings {
        crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path,
                ..Default::default()
            },
        }
    }

    async fn spawn_mock_openai_stream_server() -> String {
        async fn handler(headers: HeaderMap, Json(body): Json<Value>) -> axum::response::Response {
            assert_eq!(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer mock-upstream-key")
            );
            assert_eq!(body["stream"], json!(true));

            (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                concat!(
                    "data: {\"id\":\"stream-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"stream-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"mock stream ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":6,\"completion_tokens\":5,\"total_tokens\":11}}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/chat/completions", post(handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/v1")
    }

    async fn test_stream_app_state(
        base_url: &str,
        seed_price: bool,
        pricing_mode: PricingMode,
    ) -> (tempfile::TempDir, Arc<AppState>, String) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let mut settings = test_settings(db_path.to_string_lossy().to_string());
        settings.server.pricing_mode = pricing_mode;

        logger
            .insert_provider(&Provider {
                name: "p1".into(),
                display_name: None,
                collection: crate::config::settings::DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::OpenAI,
                api_type_raw: None,
                base_url: base_url.into(),
                api_keys: Vec::new(),
                models_endpoint: None,
                provider_config: ProviderConfig::default(),
                enabled: true,
                created_at: None,
                updated_at: None,
            })
            .await
            .unwrap();
        logger
            .add_provider_key(
                "p1",
                "mock-upstream-key",
                &settings.logging.key_log_strategy,
            )
            .await
            .unwrap();
        if seed_price {
            logger
                .upsert_model_price("p1", "m1", 1.0, 1.0, Some("USD"), None)
                .await
                .unwrap();
        }

        let token = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: None,
                name: Some("stream-token".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        (dir, app_state, token.token)
    }

    async fn invoke_stream_and_collect_text(
        app_state: Arc<AppState>,
        client_token: &str,
        model: &str,
    ) -> Result<String, crate::error::GatewayError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {client_token}")).unwrap(),
        );

        let req: ChatCompletionRequest = serde_json::from_value(json!({
            "model": model,
            "messages": [{"role":"user","content":"hi"}],
            "stream": true
        }))
        .unwrap();

        let response = stream_chat_completions(
            State(app_state),
            headers,
            Json(GatewayChatCompletionRequest {
                request: req,
                top_k: None,
            }),
        )
        .await?;

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Ok(String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn stream_data_lines(body: &str) -> Vec<&str> {
        body.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect()
    }

    #[test]
    fn explicit_non_stream_only_providers_have_clear_streaming_message() {
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::AzureOpenAI)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::GoogleGemini)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::Cohere).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::AwsClaude).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::VertexAI).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::MiniMax).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::TencentHunyuan)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::ThreeSixtyZhinao)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::StepFun).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::OpenAI).is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::BaiduErnie)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::BaiduErnieV2)
                .is_none()
        );
        assert!(
            runtime_streaming_unsupported_message(crate::config::ProviderType::XfSpark).is_none()
        );
        assert!(runtime_streaming_unsupported_message(crate::config::ProviderType::Yi).is_none());
    }

    #[tokio::test]
    async fn missing_price_strict_mode_rejects_stream_chat() {
        let base_url = spawn_mock_openai_stream_server().await;
        let (_dir, app_state, token) =
            test_stream_app_state(&base_url, false, PricingMode::Strict).await;

        let err = invoke_stream_and_collect_text(app_state.clone(), &token, "m1")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("model price not set"));

        let logs = app_state.log_store.get_recent_logs(5).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].amount_spent, None);
    }

    #[tokio::test]
    async fn missing_price_allow_missing_allows_stream_chat_without_amount() {
        let base_url = spawn_mock_openai_stream_server().await;
        let (_dir, app_state, token) =
            test_stream_app_state(&base_url, false, PricingMode::AllowMissing).await;

        let body = invoke_stream_and_collect_text(app_state.clone(), &token, "m1")
            .await
            .unwrap();
        assert_eq!(stream_data_lines(&body).last().copied(), Some("[DONE]"));
        assert!(body.contains("mock stream ok"));

        let updated = app_state
            .token_store
            .get_token(&token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.amount_spent, 0.0);
        assert_eq!(updated.total_tokens_spent, 11);

        let logs = app_state.log_store.get_recent_logs(5).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status_code, 200);
        assert_eq!(logs[0].amount_spent, None);
        assert_eq!(logs[0].total_tokens, Some(11));
    }

    #[tokio::test]
    async fn user_balance_depleted_rejects_stream_and_disables_tokens() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = test_settings(db_path.to_string_lossy().to_string());
        // Provider selection for streaming happens before token validation: seed a provider + key.
        logger
            .insert_provider(&Provider {
                name: "p1".into(),
                display_name: None,
                collection: crate::config::settings::DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::OpenAI,
                api_type_raw: None,
                base_url: "http://localhost".into(),
                api_keys: Vec::new(),
                models_endpoint: None,
                provider_config: crate::config::settings::ProviderConfig::default(),
                enabled: true,
                created_at: None,
                updated_at: None,
            })
            .await
            .unwrap();
        logger
            .add_provider_key("p1", "sk-test", &settings.logging.key_log_strategy)
            .await
            .unwrap();

        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        let user = logger
            .create_user(CreateUserPayload {
                first_name: Some("U".into()),
                last_name: Some("1".into()),
                username: None,
                email: "u1@example.com".into(),
                phone_number: None,
                password: None,
                status: UserStatus::Active,
                role: UserRole::Admin,
                is_anonymous: false,
            })
            .await
            .unwrap();

        let t1 = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("t1".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let _t2 = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("t2".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", t1.token)).unwrap(),
        );

        let req: ChatCompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "m1",
            "messages": [{"role":"user","content":"hi"}],
            "stream": true
        }))
        .unwrap();

        let err = stream_chat_completions(
            State(app_state),
            headers,
            Json(GatewayChatCompletionRequest {
                request: req,
                top_k: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("余额不足"));

        let tokens = logger.list_tokens_by_user(&user.id).await.unwrap();
        assert!(!tokens.is_empty());
        assert!(tokens.iter().all(|t| !t.enabled));
    }
}
