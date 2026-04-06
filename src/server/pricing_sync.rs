use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::config::settings::{Provider, ProviderType};
use crate::error::GatewayError;
use crate::logging::{ModelPriceRecord, ModelPriceSource, ModelPriceStatus, ModelPriceUpsert};

use super::AppState;
use super::model_types;
use super::pricing::normalize_model_price_record;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PricingSyncRequest {
    pub provider: Option<String>,
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct PricingSyncReport {
    pub dry_run: bool,
    pub force: bool,
    pub synced: usize,
    pub inserted: usize,
    pub refreshed: usize,
    pub skipped: usize,
    pub manual_protected: usize,
    pub failed: usize,
    pub stale_marked: usize,
    pub providers_processed: usize,
    pub providers_failed: usize,
    pub results: Vec<ProviderPricingSyncResult>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ProviderPricingSyncResult {
    pub provider: String,
    pub fetched: usize,
    pub synced: usize,
    pub inserted: usize,
    pub refreshed: usize,
    pub skipped: usize,
    pub manual_protected: usize,
    pub failed: usize,
    pub stale_marked: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedAutoPrice {
    provider: String,
    model: String,
    prompt_price_per_million: f64,
    completion_price_per_million: f64,
    currency: Option<String>,
    model_type: Option<String>,
    source: ModelPriceSource,
    status: ModelPriceStatus,
    synced_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy)]
struct StaticPriceDefinition {
    model: &'static str,
    prompt_price_per_million: f64,
    completion_price_per_million: f64,
    currency: &'static str,
    model_type: &'static str,
}

const OPENAI_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "gpt-4o-mini",
        prompt_price_per_million: 0.15,
        completion_price_per_million: 0.60,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gpt-4o",
        prompt_price_per_million: 2.50,
        completion_price_per_million: 10.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "o1-mini",
        prompt_price_per_million: 3.0,
        completion_price_per_million: 12.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gpt-5.4",
        prompt_price_per_million: 2.50,
        completion_price_per_million: 15.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gpt-5.4-mini",
        prompt_price_per_million: 0.75,
        completion_price_per_million: 4.5,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gpt-5.4-nano",
        prompt_price_per_million: 0.20,
        completion_price_per_million: 1.25,
        currency: "USD",
        model_type: "chat",
    },
];

const ANTHROPIC_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "claude-3-5-haiku-latest",
        prompt_price_per_million: 0.80,
        completion_price_per_million: 4.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-3-5-sonnet-latest",
        prompt_price_per_million: 3.0,
        completion_price_per_million: 15.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-3-opus-latest",
        prompt_price_per_million: 15.0,
        completion_price_per_million: 75.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-opus-4-6",
        prompt_price_per_million: 5.0,
        completion_price_per_million: 25.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-sonnet-4-6",
        prompt_price_per_million: 3.0,
        completion_price_per_million: 15.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-haiku-4-5",
        prompt_price_per_million: 1.0,
        completion_price_per_million: 5.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "claude-haiku-4-5-20251001",
        prompt_price_per_million: 1.0,
        completion_price_per_million: 5.0,
        currency: "USD",
        model_type: "chat",
    },
];

const GOOGLE_GEMINI_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "gemini-2.5-pro",
        prompt_price_per_million: 1.25,
        completion_price_per_million: 10.0,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gemini-2.5-flash",
        prompt_price_per_million: 0.30,
        completion_price_per_million: 2.5,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gemini-2.5-flash-lite",
        prompt_price_per_million: 0.10,
        completion_price_per_million: 0.40,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gemini-2.0-flash",
        prompt_price_per_million: 0.10,
        completion_price_per_million: 0.40,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "gemini-2.0-flash-lite",
        prompt_price_per_million: 0.075,
        completion_price_per_million: 0.30,
        currency: "USD",
        model_type: "chat",
    },
];

const DEEPSEEK_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "deepseek-chat",
        prompt_price_per_million: 0.28,
        completion_price_per_million: 0.42,
        currency: "USD",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "deepseek-reasoner",
        prompt_price_per_million: 0.28,
        completion_price_per_million: 0.42,
        currency: "USD",
        model_type: "chat",
    },
];

const ALIBABA_QWEN_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "qwen3-max",
        prompt_price_per_million: 2.5,
        completion_price_per_million: 10.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "qwen3-max-2026-01-23",
        prompt_price_per_million: 2.5,
        completion_price_per_million: 10.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "qwen3.6-plus",
        prompt_price_per_million: 2.0,
        completion_price_per_million: 12.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "qwen3.6-plus-2026-04-02",
        prompt_price_per_million: 2.0,
        completion_price_per_million: 12.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "qwen3.5-flash",
        prompt_price_per_million: 0.2,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "qwen3.5-flash-2026-02-23",
        prompt_price_per_million: 0.2,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
];

const TENCENT_HUNYUAN_PRICE_SOURCE: &[StaticPriceDefinition] = &[
    StaticPriceDefinition {
        model: "Hunyuan-T1",
        prompt_price_per_million: 1.0,
        completion_price_per_million: 4.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "hunyuan-t1",
        prompt_price_per_million: 1.0,
        completion_price_per_million: 4.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "Hunyuan-TurboS",
        prompt_price_per_million: 0.8,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "hunyuan-turbos",
        prompt_price_per_million: 0.8,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "Hunyuan-a13b",
        prompt_price_per_million: 0.5,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
    StaticPriceDefinition {
        model: "hunyuan-a13b",
        prompt_price_per_million: 0.5,
        completion_price_per_million: 2.0,
        currency: "CNY",
        model_type: "chat",
    },
];

pub(crate) async fn sync_model_prices(
    app_state: &AppState,
    request: PricingSyncRequest,
) -> Result<PricingSyncReport, GatewayError> {
    if !app_state.config.server.pricing_sync_enabled {
        return Err(GatewayError::Forbidden(
            "pricing sync is disabled by configuration".into(),
        ));
    }

    let now = Utc::now();
    let ttl_hours = i64::from(app_state.config.server.pricing_sync_default_ttl_hours);
    let expires_at = now + Duration::hours(ttl_hours);

    let providers = providers_for_request(app_state, request.provider.as_deref()).await?;
    let mut report = PricingSyncReport {
        dry_run: request.dry_run,
        force: request.force,
        ..Default::default()
    };

    for provider in providers {
        let result = sync_provider_prices(app_state, &provider, &request, now, expires_at).await;
        report.providers_processed += 1;
        report.synced += result.synced;
        report.inserted += result.inserted;
        report.refreshed += result.refreshed;
        report.skipped += result.skipped;
        report.manual_protected += result.manual_protected;
        report.failed += result.failed;
        report.stale_marked += result.stale_marked;
        if !result.errors.is_empty() {
            report.providers_failed += 1;
        }
        report.results.push(result);
    }

    Ok(report)
}

async fn providers_for_request(
    app_state: &AppState,
    provider_name: Option<&str>,
) -> Result<Vec<Provider>, GatewayError> {
    let providers = app_state
        .providers
        .list_providers()
        .await
        .map_err(GatewayError::Db)?;
    if let Some(provider_name) = provider_name {
        let provider = providers
            .into_iter()
            .find(|provider| provider.name == provider_name)
            .ok_or_else(|| {
                GatewayError::NotFound(format!("provider '{}' not found", provider_name))
            })?;
        Ok(vec![provider])
    } else {
        Ok(providers)
    }
}

async fn sync_provider_prices(
    app_state: &AppState,
    provider: &Provider,
    request: &PricingSyncRequest,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> ProviderPricingSyncResult {
    let mut result = ProviderPricingSyncResult {
        provider: provider.name.clone(),
        ..Default::default()
    };

    let source_entries = match fetch_price_source(provider) {
        Ok(entries) => entries,
        Err(err) => {
            result.failed += 1;
            result.errors.push(err);
            return result;
        }
    };

    let normalized_prices =
        match normalize_price_entries(app_state, provider, source_entries, now, expires_at).await {
            Ok(prices) => prices,
            Err(err) => {
                result.failed += 1;
                result.errors.push(err);
                return result;
            }
        };
    result.fetched = normalized_prices.len();

    let existing_prices = match app_state
        .log_store
        .list_model_prices(Some(&provider.name))
        .await
    {
        Ok(records) => records,
        Err(err) => {
            result.failed += 1;
            result.errors.push(err.to_string());
            return result;
        }
    };
    let mut existing_by_model = BTreeMap::<String, ModelPriceRecord>::new();
    for record in existing_prices {
        existing_by_model.insert(record.model.clone(), record);
    }
    let fetched_models: BTreeSet<_> = normalized_prices
        .iter()
        .map(|price| price.model.clone())
        .collect();

    for price in normalized_prices {
        match existing_by_model.get(&price.model).cloned() {
            Some(record) if record.source == ModelPriceSource::Manual => {
                result.manual_protected += 1;
            }
            Some(record) => {
                let normalized_record = normalize_model_price_record(record);
                if !request.force
                    && normalized_record.status == ModelPriceStatus::Active
                    && normalized_record
                        .expires_at
                        .map(|value| value > now)
                        .unwrap_or(false)
                    && auto_price_matches(&normalized_record, &price)
                {
                    result.skipped += 1;
                    continue;
                }

                if let Err(err) = persist_auto_price(app_state, request.dry_run, price).await {
                    result.failed += 1;
                    result.errors.push(err.to_string());
                    continue;
                }
                result.refreshed += 1;
                result.synced += 1;
            }
            None => {
                if let Err(err) = persist_auto_price(app_state, request.dry_run, price).await {
                    result.failed += 1;
                    result.errors.push(err.to_string());
                    continue;
                }
                result.inserted += 1;
                result.synced += 1;
            }
        }
    }

    for record in existing_by_model.into_values() {
        if record.source != ModelPriceSource::Auto || fetched_models.contains(&record.model) {
            continue;
        }

        let normalized_record = normalize_model_price_record(record.clone());
        if normalized_record.status == ModelPriceStatus::Stale
            && normalized_record
                .expires_at
                .map(|value| value <= now)
                .unwrap_or(true)
        {
            continue;
        }

        let stale_price = NormalizedAutoPrice {
            provider: record.provider,
            model: record.model,
            prompt_price_per_million: record.prompt_price_per_million,
            completion_price_per_million: record.completion_price_per_million,
            currency: record.currency,
            model_type: record.model_type,
            source: ModelPriceSource::Auto,
            status: ModelPriceStatus::Stale,
            synced_at: record.synced_at,
            expires_at: Some(now),
        };

        if let Err(err) = persist_auto_price(app_state, request.dry_run, stale_price).await {
            result.failed += 1;
            result.errors.push(err.to_string());
            continue;
        }
        result.stale_marked += 1;
    }

    result
}

fn fetch_price_source(provider: &Provider) -> Result<&'static [StaticPriceDefinition], String> {
    match provider.api_type {
        ProviderType::OpenAI => Ok(OPENAI_PRICE_SOURCE),
        ProviderType::Anthropic => Ok(ANTHROPIC_PRICE_SOURCE),
        ProviderType::GoogleGemini => Ok(GOOGLE_GEMINI_PRICE_SOURCE),
        ProviderType::DeepSeek => Ok(DEEPSEEK_PRICE_SOURCE),
        ProviderType::AlibabaQwen => Ok(ALIBABA_QWEN_PRICE_SOURCE),
        ProviderType::TencentHunyuan => Ok(TENCENT_HUNYUAN_PRICE_SOURCE),
        other => Err(format!(
            "provider '{}' with api_type '{}' is not supported by the built-in pricing sync source",
            provider.name,
            other.as_str()
        )),
    }
}

async fn normalize_price_entries(
    app_state: &AppState,
    provider: &Provider,
    source_entries: &'static [StaticPriceDefinition],
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<Vec<NormalizedAutoPrice>, String> {
    let cached_models = app_state
        .model_cache
        .get_cached_models(Some(&provider.name))
        .await
        .map_err(|err| err.to_string())?;
    if cached_models.is_empty() {
        return Err(format!(
            "provider '{}' has no cached models; refresh model cache before syncing prices",
            provider.name
        ));
    }

    let cached_model_ids = cached_models
        .into_iter()
        .map(|model| model.id)
        .collect::<HashSet<_>>();
    let mut out = Vec::new();
    for entry in source_entries {
        if !cached_model_ids.contains(entry.model) {
            continue;
        }
        if !entry.prompt_price_per_million.is_finite()
            || !entry.completion_price_per_million.is_finite()
            || entry.prompt_price_per_million < 0.0
            || entry.completion_price_per_million < 0.0
        {
            return Err(format!(
                "invalid built-in price payload for provider '{}' model '{}'",
                provider.name, entry.model
            ));
        }

        let normalized_types = model_types::normalize_model_types(Some(entry.model_type), None)
            .map_err(|err| err.to_string())?;
        let storage_model_type = model_types::model_types_to_storage(normalized_types.as_deref());

        out.push(NormalizedAutoPrice {
            provider: provider.name.clone(),
            model: entry.model.to_string(),
            prompt_price_per_million: entry.prompt_price_per_million,
            completion_price_per_million: entry.completion_price_per_million,
            currency: Some(entry.currency.to_string()),
            model_type: storage_model_type,
            source: ModelPriceSource::Auto,
            status: ModelPriceStatus::Active,
            synced_at: Some(now),
            expires_at: Some(expires_at),
        });
    }

    Ok(out)
}

fn auto_price_matches(record: &ModelPriceRecord, price: &NormalizedAutoPrice) -> bool {
    record.prompt_price_per_million == price.prompt_price_per_million
        && record.completion_price_per_million == price.completion_price_per_million
        && record.currency == price.currency
        && record.model_type == price.model_type
}

async fn persist_auto_price(
    app_state: &AppState,
    dry_run: bool,
    price: NormalizedAutoPrice,
) -> Result<(), GatewayError> {
    if dry_run {
        return Ok(());
    }
    app_state
        .log_store
        .upsert_model_price(ModelPriceUpsert {
            provider: price.provider,
            model: price.model,
            prompt_price_per_million: price.prompt_price_per_million,
            completion_price_per_million: price.completion_price_per_million,
            currency: price.currency,
            model_type: price.model_type,
            source: price.source,
            status: price.status,
            synced_at: price.synced_at,
            expires_at: price.expires_at,
        })
        .await
        .map_err(GatewayError::Db)
}

#[cfg(test)]
mod tests {
    use super::{PricingSyncRequest, fetch_price_source, sync_model_prices};
    use crate::config::BalanceStrategy;
    use crate::config::settings::{
        DEFAULT_PROVIDER_COLLECTION, LoadBalancing, LoggingConfig, Provider, ProviderConfig,
        ProviderType, ServerConfig,
    };
    use crate::logging::{DatabaseLogger, ModelPriceSource, ModelPriceStatus, ModelPriceUpsert};
    use crate::providers::openai::Model;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use chrono::{Duration, Utc};
    use std::sync::Arc;
    use tempfile::tempdir;

    struct Harness {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
    }

    fn test_settings(db_path: String) -> crate::config::Settings {
        crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig {
                pricing_sync_default_ttl_hours: 24,
                ..Default::default()
            },
            logging: LoggingConfig {
                database_path: db_path,
                ..Default::default()
            },
        }
    }

    async fn harness() -> Harness {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings = test_settings(db_path.to_string_lossy().to_string());
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

        logger
            .insert_provider(&Provider {
                name: "openai-provider".into(),
                display_name: Some("OpenAI Provider".into()),
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
            .insert_provider(&Provider {
                name: "unsupported-provider".into(),
                display_name: Some("Unsupported Provider".into()),
                collection: DEFAULT_PROVIDER_COLLECTION.into(),
                api_type: ProviderType::Custom,
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
                "openai-provider",
                &[
                    Model {
                        id: "gpt-4o-mini".into(),
                        object: "model".into(),
                        created: 0,
                        owned_by: "openai".into(),
                        display_name: None,
                    },
                    Model {
                        id: "unknown-model".into(),
                        object: "model".into(),
                        created: 0,
                        owned_by: "openai".into(),
                        display_name: None,
                    },
                ],
            )
            .await
            .unwrap();
        logger
            .cache_models(
                "unsupported-provider",
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

        let state = Arc::new(AppState {
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

        Harness { _dir: dir, state }
    }

    #[tokio::test]
    async fn sync_keeps_manual_prices_unchanged() {
        let h = harness().await;
        h.state
            .log_store
            .upsert_model_price(ModelPriceUpsert::manual(
                "openai-provider",
                "gpt-4o-mini",
                9.0,
                10.0,
                Some("USD".into()),
                Some("chat".into()),
            ))
            .await
            .unwrap();

        let report = sync_model_prices(
            &h.state,
            PricingSyncRequest {
                provider: Some("openai-provider".into()),
                dry_run: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.manual_protected, 1);
        assert_eq!(report.synced, 0);

        let record = h
            .state
            .log_store
            .get_model_price("openai-provider", "gpt-4o-mini")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.source, ModelPriceSource::Manual);
        assert_eq!(record.prompt_price_per_million, 9.0);
        assert_eq!(record.completion_price_per_million, 10.0);
    }

    #[tokio::test]
    async fn sync_refreshes_stale_auto_prices() {
        let h = harness().await;
        let old_synced_at = Utc::now() - Duration::hours(48);
        let old_expires_at = Utc::now() - Duration::hours(24);
        h.state
            .log_store
            .upsert_model_price(ModelPriceUpsert {
                provider: "openai-provider".into(),
                model: "gpt-4o-mini".into(),
                prompt_price_per_million: 0.05,
                completion_price_per_million: 0.10,
                currency: Some("USD".into()),
                model_type: Some("chat".into()),
                source: ModelPriceSource::Auto,
                status: ModelPriceStatus::Stale,
                synced_at: Some(old_synced_at),
                expires_at: Some(old_expires_at),
            })
            .await
            .unwrap();

        let report = sync_model_prices(
            &h.state,
            PricingSyncRequest {
                provider: Some("openai-provider".into()),
                dry_run: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.refreshed, 1);
        let record = h
            .state
            .log_store
            .get_model_price("openai-provider", "gpt-4o-mini")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.source, ModelPriceSource::Auto);
        assert_eq!(record.status, ModelPriceStatus::Active);
        assert_eq!(record.prompt_price_per_million, 0.15);
        assert_eq!(record.completion_price_per_million, 0.60);
        assert!(record.synced_at.unwrap() > old_synced_at);
        assert!(record.expires_at.unwrap() > old_expires_at);
    }

    #[tokio::test]
    async fn sync_inserts_missing_model_prices() {
        let h = harness().await;

        let report = sync_model_prices(
            &h.state,
            PricingSyncRequest {
                provider: Some("openai-provider".into()),
                dry_run: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.inserted, 1);
        let record = h
            .state
            .log_store
            .get_model_price("openai-provider", "gpt-4o-mini")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.source, ModelPriceSource::Auto);
        assert_eq!(record.status, ModelPriceStatus::Active);
        assert_eq!(record.currency.as_deref(), Some("USD"));
    }

    #[tokio::test]
    async fn sync_failure_does_not_change_existing_records() {
        let h = harness().await;
        h.state
            .log_store
            .upsert_model_price(ModelPriceUpsert::manual(
                "unsupported-provider",
                "gpt-4o-mini",
                7.0,
                8.0,
                Some("USD".into()),
                Some("chat".into()),
            ))
            .await
            .unwrap();

        let report = sync_model_prices(
            &h.state,
            PricingSyncRequest {
                provider: Some("unsupported-provider".into()),
                dry_run: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.providers_failed, 1);
        assert_eq!(report.failed, 1);
        let record = h
            .state
            .log_store
            .get_model_price("unsupported-provider", "gpt-4o-mini")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.source, ModelPriceSource::Manual);
        assert_eq!(record.prompt_price_per_million, 7.0);
        assert_eq!(record.completion_price_per_million, 8.0);
    }

    #[tokio::test]
    async fn sync_marks_disappeared_auto_prices_stale() {
        let h = harness().await;
        h.state
            .log_store
            .upsert_model_price(ModelPriceUpsert {
                provider: "openai-provider".into(),
                model: "retired-model".into(),
                prompt_price_per_million: 1.0,
                completion_price_per_million: 2.0,
                currency: Some("USD".into()),
                model_type: Some("chat".into()),
                source: ModelPriceSource::Auto,
                status: ModelPriceStatus::Active,
                synced_at: Some(Utc::now() - Duration::hours(5)),
                expires_at: Some(Utc::now() + Duration::hours(5)),
            })
            .await
            .unwrap();

        let report = sync_model_prices(
            &h.state,
            PricingSyncRequest {
                provider: Some("openai-provider".into()),
                dry_run: false,
                force: false,
            },
        )
        .await
        .unwrap();

        assert_eq!(report.stale_marked, 1);
        let record = h
            .state
            .log_store
            .get_model_price("openai-provider", "retired-model")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, ModelPriceStatus::Stale);
        assert_eq!(record.source, ModelPriceSource::Auto);
        assert!(record.expires_at.unwrap() <= Utc::now());
    }

    #[test]
    fn fetch_price_source_supports_new_major_providers() {
        let providers = [
            (
                ProviderType::GoogleGemini,
                "gemini-2.5-pro",
                "USD",
                1.25,
                10.0,
            ),
            (ProviderType::DeepSeek, "deepseek-chat", "USD", 0.28, 0.42),
            (ProviderType::AlibabaQwen, "qwen3-max", "CNY", 2.5, 10.0),
            (ProviderType::TencentHunyuan, "Hunyuan-T1", "CNY", 1.0, 4.0),
        ];

        for (api_type, model, currency, prompt_price, completion_price) in providers {
            let provider = Provider {
                name: format!("{}-provider", api_type.as_str()),
                display_name: None,
                collection: DEFAULT_PROVIDER_COLLECTION.into(),
                api_type,
                api_type_raw: None,
                base_url: "https://example.com/v1".into(),
                api_keys: Vec::new(),
                models_endpoint: None,
                provider_config: ProviderConfig::default(),
                enabled: true,
                created_at: None,
                updated_at: None,
            };

            let entries = fetch_price_source(&provider).unwrap();
            let entry = entries.iter().find(|entry| entry.model == model).unwrap();
            assert_eq!(entry.currency, currency);
            assert_eq!(entry.prompt_price_per_million, prompt_price);
            assert_eq!(entry.completion_price_per_million, completion_price);
        }
    }
}
