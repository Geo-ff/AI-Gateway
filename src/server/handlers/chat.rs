use axum::http::HeaderMap;
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::provider_dispatch::{
    call_provider_with_parsed_model, select_provider_for_model,
};
use crate::server::streaming::stream_chat_completions;
use crate::server::{
    AppState,
    model_redirect::{apply_model_redirects, apply_provider_model_redirects_to_parsed_model},
    request_logging::log_chat_request,
};

fn is_openai_error_payload(v: &serde_json::Value) -> bool {
    // OpenAI-style error payload is `{ "error": { ... } }` without `choices`.
    v.get("error").is_some() && v.get("choices").is_none()
}

fn error_payload_to_chat_completion(
    provider: &str,
    effective_model: &str,
    error: &serde_json::Value,
) -> serde_json::Value {
    let created = Utc::now().timestamp().max(0) as u64;
    let id = format!("chatcmpl-error-{}", Utc::now().timestamp_millis());
    let pretty = serde_json::to_string_pretty(error).unwrap_or_else(|_| error.to_string());
    // Make the error visible in chat UIs that otherwise would show an empty assistant message.
    // Frontends that support HTML-in-Markdown can render the title in red.
    let content = format!(
        "<span style=\"color:#ef4444\">({}) provider error</span>\n\n```json\n{}\n```",
        provider, pretty
    );
    serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": effective_model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }]
    })
}

/// Chat Completions 主处理入口：
/// - 根据 `stream` 标志分流到流式或一次性请求路径
/// - 校验并加载客户端令牌，检查额度/过期/模型白名单等限制
/// - 根据模型选择具体 Provider，校验价格配置并调用上游
/// - 记录详细请求日志与 usage，用于后续统计和自动禁用超限令牌
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayChatCompletionRequest {
    #[serde(flatten)]
    pub request: ChatCompletionRequest,
    /// Top-k 采样参数（尽力而为：目前仅 Anthropic 生效）
    pub top_k: Option<u32>,
}

pub async fn chat_completions(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(gateway_req): Json<GatewayChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    let top_k = gateway_req.top_k;
    let request = gateway_req.request;
    if request.stream.unwrap_or(false) {
        let response = stream_chat_completions(State(app_state), headers, Json(request)).await?;
        Ok(response.into_response())
    } else {
        let mut request = request;
        let start_time = Utc::now();
        let requested_model = request.model.clone();
        apply_model_redirects(&mut request);
        // If request pins a provider, redirected source models should be rejected (not rewritten).
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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
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
                let ge =
                    GatewayError::Config("余额不足：密钥已失效；充值/订阅后需手动启用密钥".into());
                let code = ge.status_code().as_u16();
                crate::server::request_logging::log_simple_request(
                    &app_state,
                    start_time,
                    "POST",
                    "/v1/chat/completions",
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
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
                    crate::logging::types::REQ_TYPE_CHAT_ONCE,
                    Some(request.model.clone()),
                    None,
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(request.model.clone()),
                None,
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }

        if let Some(exp) = token.expires_at
            && Utc::now() > exp
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(request.model.clone()),
                None,
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
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(from),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
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
        let upstream_model = parsed_model.get_upstream_model_name().to_string();
        if let Ok(Some(false)) = app_state
            .log_store
            .get_model_enabled(&selected.provider.name, &upstream_model)
            .await
        {
            let ge = GatewayError::Config("model is disabled".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(upstream_model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let mut price = app_state
            .log_store
            .get_model_price(&selected.provider.name, &upstream_model)
            .await
            .map_err(GatewayError::Db)?;
        let mut effective_model_for_price = upstream_model.clone();
        if price.is_none() {
            if let Some(fallback) = redirected_from_for_price.as_deref() {
                if let Ok(p) = app_state
                    .log_store
                    .get_model_price(&selected.provider.name, fallback)
                    .await
                {
                    if p.is_some() {
                        price = p;
                        effective_model_for_price = fallback.to_string();
                    }
                }
            }
        }
        // 若客户端直接使用了重定向后的 target 模型，尝试回溯找到一个 source 模型价格（best-effort）
        if price.is_none() {
            let pairs = app_state
                .providers
                .list_model_redirects(&selected.provider.name)
                .await
                .map_err(GatewayError::Db)?;
            if !pairs.is_empty() {
                use std::collections::{HashMap, HashSet};
                let map: HashMap<String, String> = pairs.into_iter().collect();
                fn resolve_redirect_chain(
                    map: &HashMap<String, String>,
                    source_model: &str,
                    max_hops: usize,
                ) -> String {
                    let mut current = source_model.to_string();
                    let mut seen = HashSet::<String>::new();
                    for _ in 0..max_hops {
                        if !seen.insert(current.clone()) {
                            break;
                        }
                        match map.get(&current) {
                            Some(next) if next != &current => current = next.clone(),
                            _ => break,
                        }
                    }
                    current
                }

                for (source, _) in map.iter() {
                    let resolved = resolve_redirect_chain(&map, source, 16);
                    if resolved != upstream_model {
                        continue;
                    }
                    let p = app_state
                        .log_store
                        .get_model_price(&selected.provider.name, source)
                        .await
                        .map_err(GatewayError::Db)?;
                    if p.is_some() {
                        price = p;
                        effective_model_for_price = source.to_string();
                        break;
                    }
                }
            }
        }
        if price.is_none() {
            let ge = GatewayError::Config("model price not set".into());
            let code = ge.status_code().as_u16();
            crate::server::request_logging::log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/v1/chat/completions",
                crate::logging::types::REQ_TYPE_CHAT_ONCE,
                Some(upstream_model.clone()),
                Some(selected.provider.name.clone()),
                client_token_log_id.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
        let response =
            call_provider_with_parsed_model(&selected, &request, &parsed_model, top_k).await;

        // 若上游返回 OpenAI 风格错误对象（有 error 无 choices），则对前端应当返回非 2xx，
        // 否则一些对话界面会误判为成功响应并显示空 assistant 内容。
        let upstream_error_body = response
            .as_ref()
            .ok()
            .filter(|dual| is_openai_error_payload(&dual.raw))
            .map(|dual| dual.raw.clone());

        // 日志/计费：请求日志中存 token id（不落明文），但 client_tokens 用量增量更新仍需要原始 token 值
        let token_for_log = client_token.as_deref();
        let billing_model = effective_model_for_price;
        let effective_model = upstream_model;
        if let Some(body) = upstream_error_body.as_ref() {
            let response_for_log: Result<_, GatewayError> = Err(GatewayError::Config(format!(
                "upstream returned error payload: {}",
                body
            )));
            log_chat_request(
                &app_state,
                start_time,
                &billing_model,
                &requested_model,
                &effective_model,
                &selected.provider.name,
                &selected.api_key,
                token_for_log,
                &response_for_log,
            )
            .await;
        } else {
            log_chat_request(
                &app_state,
                start_time,
                &billing_model,
                &requested_model,
                &effective_model,
                &selected.provider.name,
                &selected.api_key,
                token_for_log,
                &response,
            )
            .await;
        }

        // Auto-disable token when exceeding budget (post-check)
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

        // 将原始 JSON 透传给前端，以保留 reasoning_content 等扩展字段；
        // 但若是上游错误对象（误返回 200 且无 choices），则构造一个“带错误文本的 assistant 消息”，避免对话界面出现空回复。
        if let Some(body) = upstream_error_body {
            let v =
                error_payload_to_chat_completion(&selected.provider.name, &effective_model, &body);
            return Ok(Json(v).into_response());
        }
        match response {
            Ok(dual) => Ok(Json(dual.raw).into_response()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{error_payload_to_chat_completion, is_openai_error_payload};
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use axum::Json;
    use axum::extract::State;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn openai_error_payload_detection() {
        let v = serde_json::json!({
            "error": {
                "message": "openai_error",
                "type": "bad_response_status_code",
                "param": "",
                "code": "bad_response_status_code"
            }
        });
        assert!(is_openai_error_payload(&v));

        let ok = serde_json::json!({
            "id": "chatcmpl_x",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }]
        });
        assert!(!is_openai_error_payload(&ok));
    }

    #[test]
    fn openai_error_payload_is_rendered_as_assistant_message() {
        let err = serde_json::json!({
            "error": {
                "message": "openai_error",
                "type": "bad_response_status_code",
                "param": "",
                "code": "bad_response_status_code"
            }
        });
        let v = error_payload_to_chat_completion("fox", "m1", &err);
        assert!(v.get("choices").is_some());
        let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
        assert!(content.contains("provider error"));
        assert!(content.contains("```json"));
        assert!(content.contains("openai_error"));
    }

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

    #[tokio::test]
    async fn user_balance_depleted_rejects_chat_and_disables_tokens() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = test_settings(db_path.to_string_lossy().to_string());
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

        let req: crate::providers::openai::ChatCompletionRequest =
            serde_json::from_value(serde_json::json!({
                "model": "m1",
                "messages": [{"role":"user","content":"hi"}],
                "stream": false
            }))
            .unwrap();

        let err = super::chat_completions(
            State(app_state),
            headers,
            Json(super::GatewayChatCompletionRequest {
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
