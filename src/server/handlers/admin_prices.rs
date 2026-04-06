use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::error::GatewayError;
use crate::logging::types::ProviderOpLog;
use crate::logging::{ModelPriceSource, ModelPriceStatus, ModelPriceUpsert};
use crate::server::AppState;
use crate::server::model_types;
use crate::server::pricing::{
    ModelPriceView, derive_model_price_view, model_price_view_from_record,
    normalized_price_metadata,
};
use crate::server::pricing_sync::{PricingSyncReport, PricingSyncRequest};
use crate::server::request_logging::log_simple_request;
use chrono::Utc;
use serde::Serialize;

fn normalize_price_currency(currency: Option<&str>) -> Result<Option<String>, GatewayError> {
    let Some(currency) = currency.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    match currency.to_ascii_uppercase().as_str() {
        "USD" => Ok(Some("USD".into())),
        "CNY" | "RMB" | "CNH" => Ok(Some("CNY".into())),
        other => Err(GatewayError::Config(format!(
            "unsupported price currency '{}' ; only USD and CNY are allowed",
            other
        ))),
    }
}

fn validate_non_negative_price(field: &str, value: f64) -> Result<(), GatewayError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(GatewayError::Config(format!(
            "{} must be a non-negative finite number",
            field
        )))
    }
}

async fn ensure_provider_exists(app_state: &AppState, provider: &str) -> Result<(), GatewayError> {
    if app_state
        .providers
        .provider_exists(provider)
        .await
        .map_err(GatewayError::Db)?
    {
        Ok(())
    } else {
        Err(GatewayError::NotFound(format!(
            "provider '{}' not found",
            provider
        )))
    }
}

async fn ensure_cached_model_exists(
    app_state: &AppState,
    provider: &str,
    model: &str,
) -> Result<(), GatewayError> {
    let models = crate::server::model_cache::get_cached_models_for_provider(app_state, provider)
        .await
        .map_err(GatewayError::Db)?;
    if models.iter().any(|item| item.id == model) {
        Ok(())
    } else {
        Err(GatewayError::NotFound(format!(
            "model '{}' not found under provider '{}'",
            model, provider
        )))
    }
}

async fn load_model_price_view(
    app_state: &AppState,
    provider: &str,
    model: &str,
) -> Result<ModelPriceView, GatewayError> {
    match app_state
        .log_store
        .get_model_price(provider, model)
        .await
        .map_err(GatewayError::Db)?
    {
        Some(record) => Ok(model_price_view_from_record(record)),
        None => {
            ensure_cached_model_exists(app_state, provider, model).await?;
            Ok(derive_model_price_view(provider, model, None))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpsertModelPricePayload {
    pub provider: String,
    pub model: String,
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub model_type: Option<String>,
    #[serde(default)]
    pub model_types: Option<Vec<String>>,
    #[serde(default)]
    pub source: Option<ModelPriceSource>,
    #[serde(default)]
    pub status: Option<ModelPriceStatus>,
    #[serde(default)]
    pub synced_at: Option<chrono::DateTime<Utc>>,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct SyncModelPricesPayload {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct SyncSingleModelPricePayload {
    #[serde(default)]
    pub override_manual: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SingleModelSyncOutcome {
    Synced,
    ManualProtected,
    Missing,
    StaleMarked,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SingleModelPriceSyncResponse {
    pub outcome: SingleModelSyncOutcome,
    pub price: ModelPriceView,
}

pub async fn upsert_model_price(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpsertModelPricePayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        // audit + request logs on failure
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: "model_price_upsert".to_string(),
                provider: Some(payload.provider.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/model-prices",
            "model_price_upsert",
            Some(payload.model.clone()),
            Some(payload.provider.clone()),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    if let Err(ge) = ensure_provider_exists(&app_state, &payload.provider).await {
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/model-prices",
            "model_price_upsert",
            Some(payload.model.clone()),
            Some(payload.provider.clone()),
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }
    if let Err(ge) = ensure_cached_model_exists(&app_state, &payload.provider, &payload.model).await
    {
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/model-prices",
            "model_price_upsert",
            Some(payload.model.clone()),
            Some(payload.provider.clone()),
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }
    validate_non_negative_price("prompt_price_per_million", payload.prompt_price_per_million)?;
    validate_non_negative_price(
        "completion_price_per_million",
        payload.completion_price_per_million,
    )?;
    let normalized_currency = normalize_price_currency(payload.currency.as_deref())?;
    let normalized_types = model_types::normalize_model_types(
        payload.model_type.as_deref(),
        payload.model_types.as_deref(),
    )?;
    let storage_model_type = model_types::model_types_to_storage(normalized_types.as_deref());
    let (source, status, synced_at, expires_at) = normalized_price_metadata(
        payload.source,
        payload.status,
        payload.synced_at,
        payload.expires_at,
    );
    app_state
        .log_store
        .upsert_model_price(ModelPriceUpsert {
            provider: payload.provider.clone(),
            model: payload.model.clone(),
            prompt_price_per_million: payload.prompt_price_per_million,
            completion_price_per_million: payload.completion_price_per_million,
            currency: normalized_currency.clone(),
            model_type: storage_model_type.clone(),
            source,
            status,
            synced_at,
            expires_at,
        })
        .await
        .map_err(GatewayError::Db)?;
    // Success logs
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: "model_price_upsert".into(),
            provider: Some(payload.provider.clone()),
            details: Some(
                serde_json::json!({
                    "model": payload.model,
                    "prompt_price_per_million": payload.prompt_price_per_million,
                    "completion_price_per_million": payload.completion_price_per_million,
                    "currency": normalized_currency,
                    "model_type": storage_model_type,
                    "model_types": normalized_types,
                    "source": source,
                    "status": status,
                    "synced_at": synced_at,
                    "expires_at": expires_at,
                })
                .to_string(),
            ),
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/model-prices",
        "model_price_upsert",
        Some(payload.model.clone()),
        Some(payload.provider.clone()),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;
    Ok((
        axum::http::StatusCode::OK,
        Json(load_model_price_view(&app_state, &payload.provider, &payload.model).await?),
    )
        .into_response())
}

pub async fn sync_model_prices(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SyncModelPricesPayload>,
) -> Result<Json<PricingSyncReport>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: "model_price_sync".to_string(),
                provider: payload.provider.clone(),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/model-prices/sync",
            "model_price_sync",
            None,
            payload.provider.clone(),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    let report = crate::server::pricing_sync::sync_model_prices(
        &app_state,
        PricingSyncRequest {
            provider: payload.provider.clone(),
            model: payload.model.clone(),
            override_manual: false,
            dry_run: payload.dry_run,
            force: payload.force,
        },
    )
    .await;

    match report {
        Ok(report) => {
            let _ = app_state
                .log_store
                .log_provider_op(ProviderOpLog {
                    id: None,
                    timestamp: start_time,
                    operation: "model_price_sync".into(),
                    provider: payload.provider.clone(),
                    details: Some(serde_json::to_string(&report).unwrap_or_else(|_| "{}".into())),
                })
                .await;
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/admin/model-prices/sync",
                "model_price_sync",
                None,
                payload.provider,
                provided_token.as_deref(),
                200,
                None,
            )
            .await;
            Ok(Json(report))
        }
        Err(err) => {
            let code = err.status_code().as_u16();
            let _ = app_state
                .log_store
                .log_provider_op(ProviderOpLog {
                    id: None,
                    timestamp: start_time,
                    operation: "model_price_sync".into(),
                    provider: payload.provider.clone(),
                    details: Some(err.to_string()),
                })
                .await;
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/admin/model-prices/sync",
                "model_price_sync",
                None,
                payload.provider,
                provided_token.as_deref(),
                code,
                Some(err.to_string()),
            )
            .await;
            Err(err)
        }
    }
}

pub async fn sync_single_model_price(
    Path((provider, model)): Path<(String, String)>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<SyncSingleModelPricePayload>,
) -> Result<Json<SingleModelPriceSyncResponse>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/admin/model-prices/{}/{}/sync", provider, model),
            "model_price_sync_single",
            Some(model.clone()),
            Some(provider.clone()),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    ensure_provider_exists(&app_state, &provider).await?;
    ensure_cached_model_exists(&app_state, &provider, &model).await?;

    let report = crate::server::pricing_sync::sync_model_prices(
        &app_state,
        PricingSyncRequest {
            provider: Some(provider.clone()),
            model: Some(model.clone()),
            override_manual: payload.override_manual,
            dry_run: false,
            force: true,
        },
    )
    .await?;

    let view = load_model_price_view(&app_state, &provider, &model).await?;
    let outcome = if report.manual_protected > 0 {
        SingleModelSyncOutcome::ManualProtected
    } else if report.synced > 0 {
        SingleModelSyncOutcome::Synced
    } else if report.stale_marked > 0 {
        SingleModelSyncOutcome::StaleMarked
    } else {
        SingleModelSyncOutcome::Missing
    };
    let response = SingleModelPriceSyncResponse {
        outcome,
        price: view,
    };
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        &format!("/admin/model-prices/{}/{}/sync", provider, model),
        "model_price_sync_single",
        Some(model.clone()),
        Some(provider.clone()),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;
    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: "model_price_sync_single".into(),
            provider: Some(provider),
            details: Some(serde_json::to_string(&response).unwrap_or_else(|_| "{}".into())),
        })
        .await;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub provider: Option<String>,
}

pub async fn list_model_prices(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ModelPriceView>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/model-prices",
            "model_price_list",
            None,
            q.provider.clone(),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    let items = app_state
        .log_store
        .list_model_prices(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;
    let cached_models = app_state
        .model_cache
        .get_cached_models(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;

    let mut by_key = std::collections::BTreeMap::<(String, String), ModelPriceView>::new();
    for item in items {
        by_key.insert(
            (item.provider.clone(), item.model.clone()),
            model_price_view_from_record(item),
        );
    }
    for model in cached_models {
        by_key
            .entry((model.provider.clone(), model.id.clone()))
            .or_insert_with(|| derive_model_price_view(&model.provider, &model.id, None));
    }
    let out: Vec<_> = by_key.into_values().collect();
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/model-prices",
        "model_price_list",
        None,
        q.provider.clone(),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;
    Ok(Json(out))
}

pub async fn get_model_price(
    Path((provider, model)): Path<(String, String)>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ModelPriceView>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/admin/model-prices/{}/{}", provider, model),
            "model_price_get",
            Some(model.clone()),
            Some(provider.clone()),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }
    match app_state
        .log_store
        .get_model_price(&provider, &model)
        .await
        .map_err(GatewayError::Db)?
    {
        Some(record) => Ok(Json(model_price_view_from_record(record))),
        None => {
            let cached_models = app_state
                .model_cache
                .get_cached_models(Some(&provider))
                .await
                .map_err(GatewayError::Db)?;
            if cached_models.iter().any(|item| item.id == model) {
                return Ok(Json(derive_model_price_view(&provider, &model, None)));
            }
            let ge = GatewayError::NotFound("model price not set".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &format!("/admin/model-prices/{}/{}", provider, model),
                "model_price_get",
                Some(model.clone()),
                Some(provider.clone()),
                provided_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            Err(ge)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BalanceStrategy;
    use crate::config::settings::{
        DEFAULT_PROVIDER_COLLECTION, LoadBalancing, LoggingConfig, Provider, ProviderConfig,
        ProviderType, ServerConfig,
    };
    use crate::logging::DatabaseLogger;
    use crate::providers::openai::Model;
    use crate::server::login::LoginManager;
    use crate::server::storage_traits::{AdminPublicKeyRecord, LoginStore, TuiSessionRecord};
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use chrono::Duration;
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
        token: String,
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

        logger
            .insert_provider(&Provider {
                name: "p1".into(),
                display_name: Some("Provider 1".into()),
                collection: DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::OpenAI,
                api_type_raw: None,
                base_url: "https://example.com/v1".into(),
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
            .cache_models(
                "p1",
                &[Model {
                    id: "m1".into(),
                    object: "model".into(),
                    created: 0,
                    owned_by: "openai".into(),
                    display_name: None,
                }],
            )
            .await
            .unwrap();
        logger
            .insert_provider(&Provider {
                name: "p_sync".into(),
                display_name: Some("Sync Provider".into()),
                collection: DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::OpenAI,
                api_type_raw: None,
                base_url: "https://api.openai.com/v1".into(),
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
            .cache_models(
                "p_sync",
                &[Model {
                    id: "gpt-4o-mini".into(),
                    object: "model".into(),
                    created: 0,
                    owned_by: "openai".into(),
                    display_name: None,
                }],
            )
            .await
            .unwrap();
        logger
            .insert_provider(&Provider {
                name: "p_sync_moonshot".into(),
                display_name: Some("Moonshot Sync Provider".into()),
                collection: DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::Moonshot,
                api_type_raw: None,
                base_url: "https://api.moonshot.cn/v1".into(),
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
            .cache_models(
                "p_sync_moonshot",
                &[Model {
                    id: "kimi-k2-0711-preview".into(),
                    object: "model".into(),
                    created: 0,
                    owned_by: "moonshot".into(),
                    display_name: None,
                }],
            )
            .await
            .unwrap();

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
                expires_at: now + Duration::hours(1),
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
            subscription_store: logger,
        });

        Harness {
            _dir: dir,
            state: app_state,
            token,
        }
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );
        headers
    }

    #[test]
    fn upsert_payload_deserializes_missing_metadata_as_none() {
        let payload: UpsertModelPricePayload = serde_json::from_value(serde_json::json!({
            "provider": "p1",
            "model": "m1",
            "prompt_price_per_million": 1.0,
            "completion_price_per_million": 2.0
        }))
        .unwrap();

        assert_eq!(payload.source, None);
        assert_eq!(payload.status, None);
        assert_eq!(payload.synced_at, None);
        assert_eq!(payload.expires_at, None);
    }

    #[test]
    fn sync_payload_defaults_flags_to_false() {
        let payload: SyncModelPricesPayload = serde_json::from_value(serde_json::json!({
            "provider": "p_sync"
        }))
        .unwrap();

        assert_eq!(payload.provider.as_deref(), Some("p_sync"));
        assert_eq!(payload.model, None);
        assert!(!payload.dry_run);
        assert!(!payload.force);
    }

    #[tokio::test]
    async fn admin_upsert_defaults_to_manual_active() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let response = upsert_model_price(
            State(h.state.clone()),
            headers.clone(),
            Json(UpsertModelPricePayload {
                provider: "p1".into(),
                model: "m1".into(),
                prompt_price_per_million: 1.0,
                completion_price_per_million: 2.0,
                currency: Some("USD".into()),
                model_type: Some("chat".into()),
                model_types: None,
                source: None,
                status: None,
                synced_at: None,
                expires_at: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let record = h
            .state
            .log_store
            .get_model_price("p1", "m1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.source, ModelPriceSource::Manual);
        assert_eq!(record.status, ModelPriceStatus::Active);
        assert_eq!(record.synced_at, None);
    }

    #[tokio::test]
    async fn admin_get_model_price_returns_missing_for_cached_model_without_price() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let Json(view) = get_model_price(
            Path(("p1".to_string(), "m1".to_string())),
            State(h.state),
            headers,
        )
        .await
        .unwrap();

        assert_eq!(view.status, ModelPriceStatus::Missing);
        assert_eq!(view.source, None);
        assert_eq!(view.prompt_price_per_million, None);
    }

    #[tokio::test]
    async fn admin_list_model_prices_includes_missing_cached_models() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let Json(items) = list_model_prices(
            State(h.state),
            headers,
            Query(ListQuery {
                provider: Some("p1".into()),
            }),
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].provider, "p1");
        assert_eq!(items[0].model, "m1");
        assert_eq!(items[0].status, ModelPriceStatus::Missing);
    }

    #[tokio::test]
    async fn admin_sync_model_prices_dry_run_does_not_write() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let Json(report) = sync_model_prices(
            State(h.state.clone()),
            headers,
            Json(SyncModelPricesPayload {
                provider: Some("p_sync".into()),
                model: None,
                dry_run: true,
                force: false,
            }),
        )
        .await
        .unwrap();

        assert!(report.dry_run);
        assert_eq!(report.inserted, 1);
        let record = h
            .state
            .log_store
            .get_model_price("p_sync", "gpt-4o-mini")
            .await
            .unwrap();
        assert!(record.is_none());
    }

    #[tokio::test]
    async fn admin_sync_single_model_returns_latest_view() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let Json(response) = sync_single_model_price(
            Path(("p_sync".to_string(), "gpt-4o-mini".to_string())),
            State(h.state.clone()),
            headers,
            Json(SyncSingleModelPricePayload::default()),
        )
        .await
        .unwrap();

        assert_eq!(response.outcome, SingleModelSyncOutcome::Synced);
        let view = response.price;
        assert_eq!(view.provider, "p_sync");
        assert_eq!(view.model, "gpt-4o-mini");
        assert_eq!(view.source, Some(ModelPriceSource::Auto));
        assert_eq!(view.status, ModelPriceStatus::Active);
        assert_eq!(view.currency.as_deref(), Some("USD"));
        assert!(view.synced_at.is_some());
        assert!(view.expires_at.is_some());
    }

    #[tokio::test]
    async fn admin_sync_single_model_supports_moonshot_provider() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let Json(response) = sync_single_model_price(
            Path((
                "p_sync_moonshot".to_string(),
                "kimi-k2-0711-preview".to_string(),
            )),
            State(h.state.clone()),
            headers,
            Json(SyncSingleModelPricePayload::default()),
        )
        .await
        .unwrap();

        assert_eq!(response.outcome, SingleModelSyncOutcome::Synced);
        let view = response.price;
        assert_eq!(view.provider, "p_sync_moonshot");
        assert_eq!(view.model, "kimi-k2-0711-preview");
        assert_eq!(view.source, Some(ModelPriceSource::Auto));
        assert_eq!(view.status, ModelPriceStatus::Active);
        assert_eq!(view.currency.as_deref(), Some("CNY"));
        assert_eq!(view.prompt_price_per_million, Some(4.0));
        assert_eq!(view.completion_price_per_million, Some(16.0));
        assert!(view.synced_at.is_some());
        assert!(view.expires_at.is_some());
    }

    #[tokio::test]
    async fn admin_sync_single_model_reports_manual_protected() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        h.state
            .log_store
            .upsert_model_price(ModelPriceUpsert::manual(
                "p_sync",
                "gpt-4o-mini",
                75.0,
                75.0,
                Some("USD".into()),
                Some("chat".into()),
            ))
            .await
            .unwrap();

        let Json(response) = sync_single_model_price(
            Path(("p_sync".to_string(), "gpt-4o-mini".to_string())),
            State(h.state.clone()),
            headers,
            Json(SyncSingleModelPricePayload::default()),
        )
        .await
        .unwrap();

        assert_eq!(response.outcome, SingleModelSyncOutcome::ManualProtected);
        assert_eq!(response.price.source, Some(ModelPriceSource::Manual));
        assert_eq!(response.price.prompt_price_per_million, Some(75.0));
        assert_eq!(response.price.synced_at, None);
    }
}
