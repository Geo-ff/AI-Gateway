use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::auth::{AdminIdentity, require_superadmin};
use crate::error::GatewayError;
use crate::logging::time::BEIJING_OFFSET;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::mask_key;

const DEFAULT_WINDOW_MINUTES: i64 = 60;
const MAX_WINDOW_MINUTES: i64 = 7 * 24 * 60;
const TARGET_METHOD: &str = "POST";
const TARGET_PATH: &str = "/v1/chat/completions";

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKeyStatsRange {
    Hour,
    Day,
    Week,
    SinceCreated,
}

#[derive(Debug, Deserialize)]
pub struct ProviderKeyStatsQuery {
    #[serde(default)]
    pub range: Option<ProviderKeyStatsRange>,
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProviderKeyStatsItem {
    pub masked_key: String,
    pub total_requests: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub availability_rate: u32,
}

#[derive(Debug, Serialize)]
pub struct ProviderKeyStatsResponse {
    pub window_minutes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub keys: Vec<ProviderKeyStatsItem>,
    pub generated_at: String,
}

fn identity_label(identity: &AdminIdentity) -> &'static str {
    match identity {
        AdminIdentity::Jwt(_) => "jwt",
        AdminIdentity::TuiSession(_) => "tui_session",
        AdminIdentity::WebSession(_) => "web_session",
    }
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn start_of_day_utc(date: NaiveDate) -> DateTime<Utc> {
    BEIJING_OFFSET
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn end_of_day_exclusive_utc(date: NaiveDate) -> DateTime<Utc> {
    start_of_day_utc(date) + Duration::days(1)
}

fn compute_availability_rate(total: u64, success: u64) -> u32 {
    if total == 0 {
        return 100;
    }
    let rate = (success as f64) * 100.0 / (total as f64);
    rate.round().clamp(0.0, 100.0) as u32
}

pub async fn provider_key_stats(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ProviderKeyStatsQuery>,
) -> Result<Json<ProviderKeyStatsResponse>, GatewayError> {
    let identity = require_superadmin(&headers, &app_state).await?;

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let (window_minutes, start_date, end_date, keys) = match q.range {
        Some(ProviderKeyStatsRange::SinceCreated) => {
            let until = Utc::now();
            let keys_raw = app_state
                .providers
                .list_provider_keys_raw_with_created_at(
                    &provider_name,
                    &app_state.config.logging.key_log_strategy,
                )
                .await
                .map_err(GatewayError::Db)?;

            let mut keys = Vec::with_capacity(keys_raw.len());
            for entry in keys_raw {
                let masked = mask_key(&entry.value);
                let raw_rows = app_state
                    .log_store
                    .aggregate_provider_key_stats(
                        TARGET_METHOD,
                        TARGET_PATH,
                        &provider_name,
                        Some(entry.created_at),
                        Some(until),
                    )
                    .await
                    .map_err(GatewayError::Db)?;

                let mut total = 0u64;
                let mut success = 0u64;
                let mut failure = 0u64;
                for row in raw_rows {
                    if mask_key(&row.api_key) == masked {
                        total += row.total_requests;
                        success += row.success_count;
                        failure += row.failure_count;
                    }
                }

                keys.push(ProviderKeyStatsItem {
                    masked_key: masked,
                    total_requests: total,
                    success_count: success,
                    failure_count: failure,
                    availability_rate: compute_availability_rate(total, success),
                });
            }

            (0, None, None, keys)
        }
        range => {
            let (since, until, window_minutes, start_date, end_date) = match range {
                Some(ProviderKeyStatsRange::Hour) => {
                    let until = Utc::now();
                    let since = until - Duration::minutes(60);
                    (since, until, 60, None, None)
                }
                Some(ProviderKeyStatsRange::Day) => {
                    let until = Utc::now();
                    let since = until - Duration::minutes(1440);
                    (since, until, 1440, None, None)
                }
                Some(ProviderKeyStatsRange::Week) => {
                    let until = Utc::now();
                    let since = until - Duration::minutes(10080);
                    (since, until, 10080, None, None)
                }
                None => {
                    if q.start_date.is_some() || q.end_date.is_some() {
                        let start = q
                            .start_date
                            .as_deref()
                            .or(q.end_date.as_deref())
                            .and_then(parse_date)
                            .ok_or_else(|| {
                                GatewayError::Config("invalid start_date/end_date".into())
                            })?;
                        let end = q
                            .end_date
                            .as_deref()
                            .or(q.start_date.as_deref())
                            .and_then(parse_date)
                            .ok_or_else(|| {
                                GatewayError::Config("invalid start_date/end_date".into())
                            })?;

                        let since = start_of_day_utc(start);
                        let until = end_of_day_exclusive_utc(end);
                        let window_minutes = (until - since).num_minutes().max(1);

                        (
                            since,
                            until,
                            window_minutes,
                            Some(start.format("%Y-%m-%d").to_string()),
                            Some(end.format("%Y-%m-%d").to_string()),
                        )
                    } else {
                        let window = q
                            .window_minutes
                            .unwrap_or(DEFAULT_WINDOW_MINUTES)
                            .clamp(1, MAX_WINDOW_MINUTES);
                        let until = Utc::now();
                        let since = until - Duration::minutes(window);
                        (since, until, window, None, None)
                    }
                }
                Some(ProviderKeyStatsRange::SinceCreated) => unreachable!("handled above"),
            };

            let raw_rows = app_state
                .log_store
                .aggregate_provider_key_stats(
                    TARGET_METHOD,
                    TARGET_PATH,
                    &provider_name,
                    Some(since),
                    Some(until),
                )
                .await
                .map_err(GatewayError::Db)?;

            // 统一用脱敏值聚合/返回（兼容历史可能写入明文的情况）
            let mut agg: HashMap<String, (u64, u64, u64)> = HashMap::new();
            for row in raw_rows {
                let k = mask_key(&row.api_key);
                let entry = agg.entry(k).or_insert((0, 0, 0));
                entry.0 += row.total_requests;
                entry.1 += row.success_count;
                entry.2 += row.failure_count;
            }

            // 以当前 provider_keys（含禁用）为准输出，避免展示已删除的 key
            let keys_raw = app_state
                .providers
                .list_provider_keys_raw(&provider_name, &app_state.config.logging.key_log_strategy)
                .await
                .map_err(GatewayError::Db)?;

            let mut keys = Vec::with_capacity(keys_raw.len());
            for entry in keys_raw {
                let masked = mask_key(&entry.value);
                let (total, success, failure) = agg.get(&masked).copied().unwrap_or((0, 0, 0));
                keys.push(ProviderKeyStatsItem {
                    masked_key: masked,
                    total_requests: total,
                    success_count: success,
                    failure_count: failure,
                    availability_rate: compute_availability_rate(total, success),
                });
            }

            (window_minutes, start_date, end_date, keys)
        }
    };

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        &format!("/admin/providers/{}/keys/stats", provider_name),
        "admin_provider_keys_stats",
        None,
        Some(provider_name),
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(ProviderKeyStatsResponse {
        window_minutes,
        start_date,
        end_date,
        keys,
        generated_at: Utc::now().to_rfc3339(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BalanceStrategy;
    use crate::config::settings::{
        LoadBalancing, LoggingConfig, Provider, ProviderType, ServerConfig,
    };
    use crate::logging::DatabaseLogger;
    use crate::logging::types::RequestLog;
    use crate::server::login::LoginManager;
    use crate::server::storage_traits::{AdminPublicKeyRecord, LoginStore, TuiSessionRecord};
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use chrono::Utc;
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

    struct Harness {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
        headers: HeaderMap,
    }

    async fn harness() -> Harness {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings = test_settings(db_path.to_str().unwrap().to_string());
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

        let fingerprint = "test-fp".to_string();
        let now = Utc::now();
        logger
            .insert_admin_key(&AdminPublicKeyRecord {
                fingerprint: fingerprint.clone(),
                public_key: vec![0u8; ed25519_dalek::PUBLIC_KEY_LENGTH],
                comment: Some("test".into()),
                enabled: true,
                created_at: now,
                last_used_at: None,
            })
            .await
            .unwrap();

        let token = "test-admin-token".to_string();
        logger
            .create_tui_session(&TuiSessionRecord {
                session_id: token.clone(),
                fingerprint,
                issued_at: now,
                expires_at: now + chrono::Duration::hours(1),
                revoked: false,
                last_code_at: None,
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

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );

        Harness {
            _dir: dir,
            state: app_state,
            headers,
        }
    }

    #[tokio::test]
    async fn provider_key_stats_aggregates_and_handles_zero_total() {
        let h = harness().await;
        let state = h.state;
        let headers = h.headers;

        state
            .providers
            .upsert_provider(&Provider {
                name: "p1".into(),
                display_name: None,
                collection: crate::config::settings::DEFAULT_PROVIDER_COLLECTION.to_string(),
                api_type: ProviderType::OpenAI,
                base_url: "http://localhost".into(),
                models_endpoint: None,
                api_keys: Vec::new(),
                enabled: true,
                created_at: None,
                updated_at: None,
            })
            .await
            .unwrap();
        state
            .providers
            .add_provider_key("p1", "sk-test-1111111111111111", &None)
            .await
            .unwrap();
        state
            .providers
            .add_provider_key("p1", "sk-test-2222222222222222", &None)
            .await
            .unwrap();

        let now = Utc::now();
        let logs = vec![
            RequestLog {
                id: None,
                timestamp: now,
                method: TARGET_METHOD.into(),
                path: TARGET_PATH.into(),
                request_type: "chat_once".into(),
                requested_model: Some("gpt-4o".into()),
                effective_model: Some("gpt-4o".into()),
                model: Some("gpt-4o".into()),
                provider: Some("p1".into()),
                api_key: Some("sk-test-1111111111111111".into()),
                client_token: None,
                user_id: None,
                amount_spent: None,
                status_code: 200,
                response_time_ms: 10,
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
                cached_tokens: None,
                reasoning_tokens: None,
                error_message: None,
            },
            RequestLog {
                id: None,
                timestamp: now,
                method: TARGET_METHOD.into(),
                path: TARGET_PATH.into(),
                request_type: "chat_once".into(),
                requested_model: Some("gpt-4o".into()),
                effective_model: Some("gpt-4o".into()),
                model: Some("gpt-4o".into()),
                provider: Some("p1".into()),
                api_key: Some("sk-test-1111111111111111".into()),
                client_token: None,
                user_id: None,
                amount_spent: None,
                status_code: 500,
                response_time_ms: 10,
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
                cached_tokens: None,
                reasoning_tokens: None,
                error_message: Some("err".into()),
            },
        ];
        for mut log in logs {
            // 兼容历史可能写入明文：写入时先脱敏
            log.api_key = log.api_key.as_deref().map(mask_key);
            state.log_store.log_request(log).await.unwrap();
        }

        let Json(resp) = provider_key_stats(
            Path("p1".into()),
            State(state),
            headers,
            Query(ProviderKeyStatsQuery {
                range: None,
                window_minutes: Some(60),
                start_date: None,
                end_date: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.keys.len(), 2);
        let k1 = resp
            .keys
            .iter()
            .find(|k| k.masked_key == mask_key("sk-test-1111111111111111"))
            .unwrap();
        assert_eq!(k1.total_requests, 2);
        assert_eq!(k1.success_count, 1);
        assert_eq!(k1.failure_count, 1);
        assert_eq!(k1.availability_rate, 50);

        let k2 = resp
            .keys
            .iter()
            .find(|k| k.masked_key == mask_key("sk-test-2222222222222222"))
            .unwrap();
        assert_eq!(k2.total_requests, 0);
        assert_eq!(k2.success_count, 0);
        assert_eq!(k2.failure_count, 0);
        assert_eq!(k2.availability_rate, 100);
    }

    #[tokio::test]
    async fn provider_key_stats_since_created_uses_key_created_at() {
        let h = harness().await;
        let state = h.state;
        let headers = h.headers;

        state
            .providers
            .upsert_provider(&Provider {
                name: "p1".into(),
                display_name: None,
                collection: crate::config::settings::DEFAULT_PROVIDER_COLLECTION.to_string(),
                api_type: ProviderType::OpenAI,
                base_url: "http://localhost".into(),
                models_endpoint: None,
                api_keys: Vec::new(),
                enabled: true,
                created_at: None,
                updated_at: None,
            })
            .await
            .unwrap();

        let now = Utc::now();
        // Log before key exists (should not be counted by since_created).
        let old_ts = now - chrono::Duration::minutes(30);
        {
            let mut log = RequestLog {
                id: None,
                timestamp: old_ts,
                method: TARGET_METHOD.into(),
                path: TARGET_PATH.into(),
                request_type: "chat_once".into(),
                requested_model: Some("gpt-4o".into()),
                effective_model: Some("gpt-4o".into()),
                model: Some("gpt-4o".into()),
                provider: Some("p1".into()),
                api_key: Some("sk-test-1111111111111111".into()),
                client_token: None,
                user_id: None,
                amount_spent: None,
                status_code: 200,
                response_time_ms: 10,
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
                cached_tokens: None,
                reasoning_tokens: None,
                error_message: None,
            };
            log.api_key = log.api_key.as_deref().map(mask_key);
            state.log_store.log_request(log).await.unwrap();
        }

        state
            .providers
            .add_provider_key("p1", "sk-test-1111111111111111", &None)
            .await
            .unwrap();

        // Log after key creation (should be included by since_created).
        let recent_ts = Utc::now();
        let mut log = RequestLog {
            id: None,
            timestamp: recent_ts,
            method: TARGET_METHOD.into(),
            path: TARGET_PATH.into(),
            request_type: "chat_once".into(),
            requested_model: Some("gpt-4o".into()),
            effective_model: Some("gpt-4o".into()),
            model: Some("gpt-4o".into()),
            provider: Some("p1".into()),
            api_key: Some("sk-test-1111111111111111".into()),
            client_token: None,
            user_id: None,
            amount_spent: None,
            status_code: 500,
            response_time_ms: 10,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            cached_tokens: None,
            reasoning_tokens: None,
            error_message: None,
        };
        log.api_key = log.api_key.as_deref().map(mask_key);
        state.log_store.log_request(log).await.unwrap();

        let Json(resp) = provider_key_stats(
            Path("p1".into()),
            State(state),
            headers,
            Query(ProviderKeyStatsQuery {
                range: Some(ProviderKeyStatsRange::SinceCreated),
                window_minutes: None,
                start_date: None,
                end_date: None,
            }),
        )
        .await
        .unwrap();

        let k1 = resp
            .keys
            .iter()
            .find(|k| k.masked_key == mask_key("sk-test-1111111111111111"))
            .unwrap();
        assert_eq!(k1.total_requests, 1);
        assert_eq!(k1.success_count, 0);
        assert_eq!(k1.failure_count, 1);
        assert_eq!(k1.availability_rate, 0);
    }
}
