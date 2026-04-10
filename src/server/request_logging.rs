use crate::admin::client_token_id_for_token;
use crate::balance::BalanceTransactionKind;
use crate::error::GatewayError;
use crate::logging::RequestLog;
use crate::logging::types::{REQ_TYPE_CHAT_ONCE, RequestLogDetailRecord};
use crate::providers::openai::types::RawAndTypedChatCompletion;
use crate::providers::openai::usage::resolved_usage;
use crate::server::AppState;
use crate::server::response_text;
use crate::server::util::mask_key;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Default)]
pub struct ChatLogContext {
    pub path: String,
    pub request_type: String,
    pub request_payload_snapshot: Option<String>,
    pub upstream_status: Option<i64>,
    pub selected_provider: Option<String>,
    pub selected_key_id: Option<String>,
    pub first_token_latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct LoggedChatRequest {
    pub log_id: Option<i64>,
    pub amount_spent: Option<f64>,
    pub response_time_ms: i64,
}

fn response_preview(response: &Result<RawAndTypedChatCompletion, GatewayError>) -> Option<String> {
    response_text::response_preview(response, 1200, 600)
}

// 记录聊天请求日志（包含响应耗时和 token 使用情况）
pub async fn log_chat_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    billing_model: &str,
    requested_model: &str,
    effective_model: &str,
    provider_name: &str,
    api_key_raw: &str,
    client_token: Option<&str>,
    response: &Result<RawAndTypedChatCompletion, GatewayError>,
    context: ChatLogContext,
) -> LoggedChatRequest {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    // 统计与日志关联使用稳定脱敏值，避免明文泄露
    let api_key = Some(mask_key(api_key_raw));
    let client_token_id = client_token.map(client_token_id_for_token);
    let usage = response
        .as_ref()
        .ok()
        .and_then(|dual| resolved_usage(&dual.raw, &dual.typed));

    // 计算本次消耗金额（仅当有价格与 usage 可用，且有 Client Token）
    let amount_spent: Option<f64> = match response {
        Ok(_) => {
            if let (Some(u), Some(_tok)) = (usage.as_ref(), client_token) {
                match app_state
                    .log_store
                    .get_model_price(provider_name, billing_model)
                    .await
                {
                    Ok(Some(record)) => {
                        let p =
                            u.prompt_tokens as f64 * record.prompt_price_per_million / 1_000_000.0;
                        let c = u.completion_tokens as f64 * record.completion_price_per_million
                            / 1_000_000.0;
                        Some(p + c)
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        Err(_) => None,
    };

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: if context.path.is_empty() {
            "/v1/chat/completions".to_string()
        } else {
            context.path.clone()
        },
        request_type: if context.request_type.is_empty() {
            REQ_TYPE_CHAT_ONCE.to_string()
        } else {
            context.request_type.clone()
        },
        requested_model: Some(requested_model.to_string()),
        effective_model: Some(effective_model.to_string()),
        model: Some(billing_model.to_string()),
        provider: Some(provider_name.to_string()),
        api_key,
        client_token: client_token_id.clone(),
        user_id: None,
        amount_spent,
        status_code: if response.is_ok() { 200 } else { 500 },
        response_time_ms,
        prompt_tokens: usage.as_ref().map(|usage| usage.prompt_tokens),
        completion_tokens: usage.as_ref().map(|usage| usage.completion_tokens),
        total_tokens: usage.as_ref().map(|usage| usage.total_tokens),
        cached_tokens: usage.as_ref().and_then(|usage| {
            usage
                .prompt_tokens_details
                .as_ref()
                .and_then(|details| details.cached_tokens)
        }),
        reasoning_tokens: usage.as_ref().and_then(|usage| {
            usage
                .completion_tokens_details
                .as_ref()
                .and_then(|details| details.reasoning_tokens)
        }),
        error_message: response.as_ref().err().map(|e| e.to_string()),
    };

    let log_id = match app_state.log_store.log_request(log).await {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::error!("Failed to log request: {}", e);
            None
        }
    };

    if let Some(request_log_id) = log_id {
        let detail = RequestLogDetailRecord {
            request_log_id,
            request_payload_snapshot: context.request_payload_snapshot,
            response_preview: response_preview(response),
            upstream_status: context.upstream_status.or(Some(if response.is_ok() {
                200
            } else {
                500
            })),
            fallback_triggered: None,
            fallback_reason: None,
            selected_provider: context
                .selected_provider
                .or_else(|| Some(provider_name.to_string())),
            selected_key_id: context
                .selected_key_id
                .or_else(|| Some(mask_key(api_key_raw))),
            first_token_latency_ms: context.first_token_latency_ms,
        };
        if let Err(e) = app_state.log_store.upsert_request_log_detail(detail).await {
            tracing::warn!("Failed to upsert request log detail: {}", e);
        }
    }

    // 增量更新 client_tokens：金额与 tokens（仅当有 usage/金额 与 Client Token 时）
    if let Some(tok) = client_token {
        // 1) update money spent (for statistics) when pricing is available
        if let Some(delta) = amount_spent {
            if let Err(e) = app_state.token_store.add_amount_spent(tok, delta).await {
                tracing::warn!("Failed to update token spent: {}", e);
            }
        }

        // 2) update token usage counters + compute tokens used for subscription billing
        let mut tokens_used: Option<i64> = None;
        if let Some(u) = usage.as_ref() {
            let prompt = u.prompt_tokens as i64;
            let completion = u.completion_tokens as i64;
            let total = u.total_tokens as i64;
            tokens_used = Some(total);
            if let Err(e) = app_state
                .token_store
                .add_usage_spent(tok, prompt, completion, total)
                .await
            {
                tracing::warn!("Failed to update token tokens: {}", e);
            }
        }

        // 3) subscription billing: user-bound tokens deduct from user.balance (unit: tokens)
        if let Some(total_tokens) = tokens_used.filter(|v| *v > 0) {
            if let Ok(Some(t)) = app_state.token_store.get_token(tok).await {
                if let Some(user_id) = t.user_id.as_deref() {
                    let delta_tokens = -(total_tokens as f64);
                    match app_state
                        .user_store
                        .add_balance(user_id, delta_tokens)
                        .await
                    {
                        Ok(Some(new_balance)) => {
                            let meta = serde_json::json!({
                                "client_token_id": t.id,
                                "path": "/v1/chat/completions",
                                "total_tokens": total_tokens,
                                "amount_spent": amount_spent,
                            })
                            .to_string();
                            if let Err(e) = app_state
                                .balance_store
                                .create_transaction(
                                    user_id,
                                    BalanceTransactionKind::Spend,
                                    delta_tokens,
                                    Some(meta),
                                )
                                .await
                            {
                                tracing::warn!("Failed to insert balance transaction: {}", e);
                            }
                            if new_balance <= 0.0 {
                                let _ = app_state
                                    .token_store
                                    .set_enabled_for_user(user_id, false)
                                    .await;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("Failed to deduct user balance: {}", e);
                        }
                    }
                }
            }
        }
    }

    LoggedChatRequest {
        log_id,
        amount_spent,
        response_time_ms,
    }
}

// 记录普通请求（不含 tokens）
#[allow(clippy::too_many_arguments)]
pub async fn log_simple_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    method: &str,
    path: &str,
    request_type: &str,
    model: Option<String>,
    provider: Option<String>,
    client_token: Option<&str>,
    status_code: u16,
    error_message: Option<String>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: method.to_string(),
        path: path.to_string(),
        request_type: request_type.to_string(),
        requested_model: model.clone(),
        effective_model: model.clone(),
        model,
        provider,
        api_key: None,
        client_token: client_token.map(|s| s.to_string()),
        user_id: None,
        amount_spent: None,
        status_code,
        response_time_ms,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
        error_message,
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::balance::BalanceStore;
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    #[tokio::test]
    async fn log_chat_request_updates_token_store_but_logs_token_id() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path.to_string_lossy().to_string(),
                ..LoggingConfig::default()
            },
        };

        let app_state = AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        };

        // model pricing needed for amount_spent
        logger
            .upsert_model_price(crate::logging::ModelPriceUpsert::manual(
                "p1",
                "m1",
                2.0,
                4.0,
                Some("USD".into()),
                None,
            ))
            .await
            .unwrap();

        let created = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: None,
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

        let raw = serde_json::json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "created": 0,
            "model": "m1",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        });
        let typed: async_openai::types::CreateChatCompletionResponse =
            serde_json::from_value(raw.clone()).unwrap();
        let dual = RawAndTypedChatCompletion { typed, raw };

        log_chat_request(
            &app_state,
            Utc::now(),
            "m1",
            "m1",
            "m1",
            "p1",
            "sk-test",
            Some(created.token.as_str()),
            &Ok(dual),
            ChatLogContext::default(),
        )
        .await;

        let updated = logger.get_token(&created.token).await.unwrap().unwrap();
        assert_eq!(updated.total_tokens_spent, 15);
        assert_eq!(updated.prompt_tokens_spent, 10);
        assert_eq!(updated.completion_tokens_spent, 5);

        let expected_spent = (10.0 * 2.0 + 5.0 * 4.0) / 1_000_000.0;
        assert!(
            approx_eq(updated.amount_spent, expected_spent, 1e-12),
            "amount_spent mismatch: got {}, expected {}",
            updated.amount_spent,
            expected_spent
        );

        // sum_spent_amount_by_client_token expects request_logs.client_token to store token id (not raw token).
        let sum = logger
            .sum_spent_amount_by_client_token(&created.id)
            .await
            .unwrap();
        assert!(approx_eq(sum, expected_spent, 1e-12));
    }

    #[tokio::test]
    async fn log_chat_request_deducts_user_balance_and_disables_tokens() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path.to_string_lossy().to_string(),
                ..LoggingConfig::default()
            },
        };

        let app_state = AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        };

        logger
            .upsert_model_price(crate::logging::ModelPriceUpsert::manual(
                "p1",
                "m1",
                1_000_000.0,
                0.0,
                Some("USD".into()),
                None,
            ))
            .await
            .unwrap();

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
        // balance unit: tokens
        logger.add_balance(&user.id, 1.0).await.unwrap().unwrap();

        let created = logger
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

        let raw = serde_json::json!({
            "id": "chatcmpl_test",
            "object": "chat.completion",
            "created": 0,
            "model": "m1",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 2, "completion_tokens": 0, "total_tokens": 2 }
        });
        let typed: async_openai::types::CreateChatCompletionResponse =
            serde_json::from_value(raw.clone()).unwrap();
        let dual = RawAndTypedChatCompletion { typed, raw };

        log_chat_request(
            &app_state,
            Utc::now(),
            "m1",
            "m1",
            "m1",
            "p1",
            "sk-test",
            Some(created.token.as_str()),
            &Ok(dual),
            ChatLogContext::default(),
        )
        .await;

        let fetched = logger.get_user(&user.id).await.unwrap().unwrap();
        assert!(fetched.balance <= 0.0);

        let tokens = logger.list_tokens_by_user(&user.id).await.unwrap();
        assert_eq!(tokens.len(), 1);
        assert!(!tokens[0].enabled);

        let txs = logger.list_transactions(&user.id, 10, 0).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].kind.as_str(), "spend");
        assert!(txs[0].amount < 0.0);
        assert!(approx_eq(txs[0].amount, -2.0, 1e-12));
    }

    #[tokio::test]
    async fn log_chat_request_uses_raw_input_output_usage_when_typed_is_empty() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("gateway.db");
        let logger = Arc::new(
            DatabaseLogger::new(db_path.to_str().unwrap())
                .await
                .unwrap(),
        );

        let settings = crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path.to_string_lossy().to_string(),
                ..LoggingConfig::default()
            },
        };

        let app_state = AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        };

        logger
            .upsert_model_price(crate::logging::ModelPriceUpsert::manual(
                "p1",
                "gpt-5.4",
                2.0,
                4.0,
                Some("USD".into()),
                None,
            ))
            .await
            .unwrap();

        let created = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: None,
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

        let raw = serde_json::json!({
            "id": "resp_123",
            "object": "response",
            "created": 0,
            "model": "gpt-5.4",
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "hello"
                }]
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 12
            }
        });
        let typed: async_openai::types::CreateChatCompletionResponse =
            serde_json::from_value(serde_json::json!({
                "id": "resp_123",
                "object": "chat.completion",
                "created": 0,
                "model": "gpt-5.4",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": null},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 0,
                    "completion_tokens": 0,
                    "total_tokens": 0
                }
            }))
            .unwrap();
        let dual = RawAndTypedChatCompletion { typed, raw };

        log_chat_request(
            &app_state,
            Utc::now(),
            "gpt-5.4",
            "gpt-5.4",
            "gpt-5.4",
            "p1",
            "sk-test",
            Some(created.token.as_str()),
            &Ok(dual),
            ChatLogContext::default(),
        )
        .await;

        let updated = logger.get_token(&created.token).await.unwrap().unwrap();
        assert_eq!(updated.prompt_tokens_spent, 10);
        assert_eq!(updated.completion_tokens_spent, 12);
        assert_eq!(updated.total_tokens_spent, 22);

        let logs = logger.get_request_logs(10, None).await.unwrap();
        assert_eq!(logs[0].prompt_tokens, Some(10));
        assert_eq!(logs[0].completion_tokens, Some(12));
        assert_eq!(logs[0].total_tokens, Some(22));
    }
}
