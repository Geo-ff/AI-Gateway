use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::balance::BalanceTransactionKind;
use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_CHAT_STREAM;
use crate::providers::openai::Usage;
use crate::server::AppState;

// 统一的流式错误日志记录函数（KISS/DRY）
pub(super) async fn log_stream_error(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    billing_model: String,
    requested_model: String,
    effective_model: String,
    provider: String,
    api_key: Option<String>,
    client_token: Option<String>,
    error_message: String,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let client_token_id = client_token
        .as_deref()
        .map(crate::admin::client_token_id_for_token);
    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
        requested_model: Some(requested_model),
        effective_model: Some(effective_model),
        model: Some(billing_model),
        provider: Some(provider),
        api_key,
        client_token: client_token_id,
        user_id: None,
        amount_spent: None,
        status_code: 500,
        response_time_ms,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
        error_message: Some(error_message),
    };
    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log streaming error: {}", e);
    }
}

// 统一的流式成功日志记录函数
pub(super) async fn log_stream_success(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    billing_model: String,
    requested_model: String,
    effective_model: String,
    provider: String,
    api_key: Option<String>,
    client_token: Option<String>,
    usage: Option<Usage>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let (prompt, completion, total, cached, reasoning) = usage
        .as_ref()
        .map(|u| {
            (
                Some(u.prompt_tokens),
                Some(u.completion_tokens),
                Some(u.total_tokens),
                u.prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens),
                u.completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens),
            )
        })
        .unwrap_or((None, None, None, None, None));
    // Compute amount_spent if possible (Client Token only)
    let amount_spent = if let Some(u) = usage.as_ref()
        && client_token.is_some()
    {
        match app_state
            .log_store
            .get_model_price(&provider, &billing_model)
            .await
        {
            Ok(Some((p_pm, c_pm, _, _))) => {
                let p = u.prompt_tokens as f64 * p_pm / 1_000_000.0;
                let c = u.completion_tokens as f64 * c_pm / 1_000_000.0;
                Some(p + c)
            }
            _ => None,
        }
    } else {
        None
    };

    let client_token_id = client_token
        .as_deref()
        .map(crate::admin::client_token_id_for_token);
    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_STREAM.to_string(),
        requested_model: Some(requested_model),
        effective_model: Some(effective_model),
        model: Some(billing_model),
        provider: Some(provider),
        api_key,
        client_token: client_token_id,
        user_id: None,
        amount_spent,
        status_code: 200,
        response_time_ms,
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cached_tokens: cached,
        reasoning_tokens: reasoning,
        error_message: None,
    };
    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log streaming request: {}", e);
    }

    // 增量更新 client_tokens：金额与 tokens（仅当有 Client Token 时）
    if let Some(tok) = client_token.as_deref() {
        if let Some(delta) = amount_spent
            && let Err(e) = app_state.token_store.add_amount_spent(tok, delta).await
        {
            tracing::warn!("Failed to update token spent: {}", e);
        }
        if let Some(u) = usage.as_ref() {
            let prompt = u.prompt_tokens as i64;
            let completion = u.completion_tokens as i64;
            let total = u.total_tokens as i64;
            if let Err(e) = app_state
                .token_store
                .add_usage_spent(tok, prompt, completion, total)
                .await
            {
                tracing::warn!("Failed to update token tokens: {}", e);
            }
        }

        // 订阅计费：绑定用户 token 只扣 user.balance（单位：tokens），不扣金额
        if let Some(u) = usage.as_ref()
            && let Ok(Some(t)) = app_state.token_store.get_token(tok).await
            && let Some(user_id) = t.user_id.as_deref()
        {
            let total_tokens = u.total_tokens as i64;
            if total_tokens > 0 {
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
                    Err(e) => tracing::warn!("Failed to deduct user balance: {}", e),
                }
            }
        }
    }

    // Auto-disable token when exceeding budget (streaming)
    if let Some(tok) = client_token.as_deref()
        && let Ok(Some(t)) = app_state.token_store.get_token(tok).await
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
}

// Extract Usage from a JSON value if fields are present (lenient across providers)
pub(super) fn parse_usage_from_value(v: &serde_json::Value) -> Option<Usage> {
    use async_openai::types::{CompletionTokensDetails, PromptTokensDetails};
    let u = v.get("usage")?;
    let prompt = u
        .get("prompt_tokens")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    let completion = u
        .get("completion_tokens")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    let total = u
        .get("total_tokens")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    let reasoning = u
        .get("completion_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|x| x.as_u64())
        .map(|x| x as u32);
    if prompt.is_none()
        && completion.is_none()
        && total.is_none()
        && cached.is_none()
        && reasoning.is_none()
    {
        return None;
    }
    Some(Usage {
        prompt_tokens: prompt.unwrap_or(0),
        completion_tokens: completion.unwrap_or(0),
        total_tokens: total.unwrap_or(prompt.unwrap_or(0) + completion.unwrap_or(0)),
        prompt_tokens_details: if cached.is_some() {
            Some(PromptTokensDetails {
                cached_tokens: cached,
                audio_tokens: None,
            })
        } else {
            None
        },
        completion_tokens_details: if reasoning.is_some() {
            Some(CompletionTokensDetails {
                reasoning_tokens: reasoning,
                audio_tokens: None,
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            })
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::balance::BalanceStore;
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::login::LoginManager;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use std::sync::Arc;
    use tempfile::tempdir;

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
    async fn stream_success_deducts_user_balance_by_tokens() {
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
        logger.add_balance(&user.id, 10_000.0).await.unwrap();

        let token = logger
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

        let start = Utc::now();
        log_stream_success(
            app_state,
            start,
            "m1".into(),
            "m1".into(),
            "m1".into(),
            "p1".into(),
            None,
            Some(token.token.clone()),
            Some(Usage {
                prompt_tokens: 400,
                completion_tokens: 600,
                total_tokens: 1000,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
        )
        .await;

        let fetched = logger.get_user(&user.id).await.unwrap().unwrap();
        assert!((fetched.balance - 9000.0).abs() < 1e-9);

        let txs = logger.list_transactions(&user.id, 10, 0).await.unwrap();
        assert!(!txs.is_empty());
        assert_eq!(txs[0].kind.as_str(), "spend");
        assert!((txs[0].amount + 1000.0).abs() < 1e-9);
    }
}
