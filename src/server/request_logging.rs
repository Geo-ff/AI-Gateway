use crate::admin::client_token_id_for_token;
use crate::error::GatewayError;
use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_CHAT_ONCE;
use crate::providers::openai::types::RawAndTypedChatCompletion;
use crate::server::AppState;
use crate::server::util::mask_key;
use chrono::{DateTime, Utc};

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
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();

    // 统计与日志关联使用稳定脱敏值，避免明文泄露
    let api_key = Some(mask_key(api_key_raw));
    let client_token_id = client_token.map(client_token_id_for_token);

    // 计算本次消耗金额（仅当有价格与 usage 可用，且有 Client Token）
    let amount_spent: Option<f64> = match response {
        Ok(dual) => {
            let usage = dual.typed.usage.as_ref();
            if let (Some(u), Some(_tok)) = (usage, client_token) {
                match app_state
                    .log_store
                    .get_model_price(provider_name, billing_model)
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
            }
        }
        Err(_) => None,
    };

    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        request_type: REQ_TYPE_CHAT_ONCE.to_string(),
        requested_model: Some(requested_model.to_string()),
        effective_model: Some(effective_model.to_string()),
        model: Some(billing_model.to_string()),
        provider: Some(provider_name.to_string()),
        api_key,
        client_token: client_token_id.clone(),
        amount_spent,
        status_code: if response.is_ok() { 200 } else { 500 },
        response_time_ms,
        prompt_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.prompt_tokens)),
        completion_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.completion_tokens)),
        total_tokens: response
            .as_ref()
            .ok()
            .and_then(|r| r.typed.usage.as_ref().map(|u| u.total_tokens)),
        cached_tokens: response.as_ref().ok().and_then(|r| {
            r.typed.usage.as_ref().and_then(|u| {
                u.prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens)
            })
        }),
        reasoning_tokens: response.as_ref().ok().and_then(|r| {
            r.typed.usage.as_ref().and_then(|u| {
                u.completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens)
            })
        }),
        error_message: response.as_ref().err().map(|e| e.to_string()),
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log request: {}", e);
    }

    // 增量更新 client_tokens：金额与 tokens（仅当有 usage/金额 与 Client Token 时）
    if let Some(tok) = client_token {
        if let Some(delta) = amount_spent
            && let Err(e) = app_state.token_store.add_amount_spent(tok, delta).await
        {
            tracing::warn!("Failed to update token spent: {}", e);
        }
        if let Ok(r) = response
            && let Some(u) = r.typed.usage.as_ref()
        {
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
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
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
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
        };

        // model pricing needed for amount_spent
        logger
            .upsert_model_price("p1", "m1", 2.0, 4.0, Some("USD"), None)
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
}
