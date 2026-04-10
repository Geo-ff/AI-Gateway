use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use crate::balance::BalanceTransactionKind;
use crate::logging::RequestLog;
use crate::logging::types::{REQ_TYPE_CHAT_STREAM, RequestLogDetailRecord};
use crate::providers::openai::Usage;
use crate::server::AppState;
use crate::server::response_text;

const STREAM_RESPONSE_PREVIEW_MAX_LEN: usize = 1200;

#[derive(Debug, Clone, Default)]
pub(super) struct StreamLogContext {
    pub request_payload_snapshot: Option<String>,
    pub response_preview: Option<String>,
    pub first_token_latency_ms: Option<i64>,
}

async fn upsert_stream_log_detail(
    app_state: &AppState,
    request_log_id: i64,
    provider: &str,
    api_key: Option<&str>,
    status_code: u16,
    context: &StreamLogContext,
) {
    let detail = RequestLogDetailRecord {
        request_log_id,
        request_payload_snapshot: context.request_payload_snapshot.clone(),
        response_preview: context.response_preview.clone(),
        upstream_status: Some(i64::from(status_code)),
        fallback_triggered: None,
        fallback_reason: None,
        selected_provider: Some(provider.to_string()),
        selected_key_id: api_key.map(str::to_string),
        first_token_latency_ms: context.first_token_latency_ms,
    };
    if let Err(error) = app_state.log_store.upsert_request_log_detail(detail).await {
        tracing::warn!("Failed to upsert streaming request log detail: {}", error);
    }
}

pub(super) fn append_response_preview_fragment(
    preview_cell: &Arc<Mutex<String>>,
    fragment: Option<String>,
) {
    let Some(fragment) = fragment else {
        return;
    };
    preview_cell.lock().unwrap().push_str(&fragment);
}

pub(super) fn context_with_stream_preview(
    context: &StreamLogContext,
    preview_cell: &Arc<Mutex<String>>,
) -> StreamLogContext {
    let mut next_context = context.clone();
    next_context.response_preview = response_text::preview_from_stream_text(
        preview_cell.lock().unwrap().clone(),
        STREAM_RESPONSE_PREVIEW_MAX_LEN,
    );
    next_context
}

pub(super) fn context_with_response_preview(
    context: &StreamLogContext,
    response_preview: Option<String>,
) -> StreamLogContext {
    let mut next_context = context.clone();
    next_context.response_preview = response_preview;
    next_context
}

pub(super) fn record_first_token_latency(
    context: &mut StreamLogContext,
    start_time: DateTime<Utc>,
) {
    if context.first_token_latency_ms.is_none() {
        context.first_token_latency_ms = Some((Utc::now() - start_time).num_milliseconds());
    }
}

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
    context: StreamLogContext,
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
        provider: Some(provider.clone()),
        api_key: api_key.clone(),
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
    match app_state.log_store.log_request(log).await {
        Ok(log_id) => {
            upsert_stream_log_detail(
                &app_state,
                log_id,
                &provider,
                api_key.as_deref(),
                500,
                &context,
            )
            .await;
        }
        Err(e) => {
            tracing::error!("Failed to log streaming error: {}", e);
        }
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
    context: StreamLogContext,
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
            Ok(Some(record)) => {
                let p = u.prompt_tokens as f64 * record.prompt_price_per_million / 1_000_000.0;
                let c =
                    u.completion_tokens as f64 * record.completion_price_per_million / 1_000_000.0;
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
        provider: Some(provider.clone()),
        api_key: api_key.clone(),
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
    match app_state.log_store.log_request(log).await {
        Ok(log_id) => {
            upsert_stream_log_detail(
                &app_state,
                log_id,
                &provider,
                api_key.as_deref(),
                200,
                &context,
            )
            .await;
        }
        Err(e) => {
            tracing::error!("Failed to log streaming request: {}", e);
        }
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
            organizations: logger.clone(),
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
            StreamLogContext::default(),
        )
        .await;

        let fetched = logger.get_user(&user.id).await.unwrap().unwrap();
        assert!((fetched.balance - 9000.0).abs() < 1e-9);

        let txs = logger.list_transactions(&user.id, 10, 0).await.unwrap();
        assert!(!txs.is_empty());
        assert_eq!(txs[0].kind.as_str(), "spend");
        assert!((txs[0].amount + 1000.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn stream_success_persists_request_snapshot_detail() {
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
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        let token = logger
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

        let snapshot = serde_json::json!({
            "kind": "chat_completions",
            "request": {
                "model": "demo/model",
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }
        })
        .to_string();

        log_stream_success(
            app_state,
            Utc::now(),
            "demo/model".into(),
            "demo/model".into(),
            "demo/model".into(),
            "demo-provider".into(),
            Some("sk-d****cret".into()),
            Some(token.token.clone()),
            Some(Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
            StreamLogContext {
                request_payload_snapshot: Some(snapshot.clone()),
                response_preview: Some("hello world".into()),
                first_token_latency_ms: Some(123),
            },
        )
        .await;

        let logs = logger
            .get_logs_by_client_token(&token.id, 10)
            .await
            .unwrap();
        let log_id = logs.first().and_then(|item| item.id).unwrap();
        let detail = logger
            .get_request_log_detail(log_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.request_payload_snapshot, Some(snapshot));
        assert_eq!(detail.selected_provider.as_deref(), Some("demo-provider"));
        assert_eq!(detail.upstream_status, Some(200));
        assert_eq!(detail.selected_key_id.as_deref(), Some("sk-d****cret"));
        assert_eq!(detail.response_preview.as_deref(), Some("hello world"));
    }
}
