pub mod handlers;
pub(crate) mod model_helpers;
pub(crate) mod model_cache;
pub(crate) mod model_parser;
pub(crate) mod provider_dispatch;
pub(crate) mod model_redirect;
pub(crate) mod request_logging;
pub(crate) mod storage_traits;
pub(crate) mod streaming;

use crate::config::Settings;
use crate::logging::DatabaseLogger;
use crate::logging::postgres_store::PgLogStore;
use crate::server::storage_traits::{ModelCache, RequestLogStore};
use crate::server::storage_traits::ProviderStore;
use axum::Router;
use std::sync::Arc;
use crate::error::Result as AppResult;
use crate::admin::{TokenStore, PgTokenStore};

#[derive(Clone)]
pub struct AppState {
    pub config: Settings,
    pub log_store: Arc<dyn RequestLogStore + Send + Sync>,
    pub model_cache: Arc<dyn ModelCache + Send + Sync>,
    pub providers: Arc<dyn ProviderStore + Send + Sync>,
    pub token_store: Arc<dyn TokenStore + Send + Sync>,
    pub admin_identity_token: String,
}

pub async fn create_app(config: Settings) -> AppResult<Router> {
    // Admin identity token generated per boot
    let admin_identity_token: String = {
        use rand::Rng;
        let rng = rand::rng();
        use rand::distr::Alphanumeric;
        rng.sample_iter(&Alphanumeric).take(56).map(char::from).collect()
    };

    // Choose stores based on Postgres availability
    let (log_store_arc, model_cache_arc, provider_store_arc, token_store): (
        Arc<dyn RequestLogStore + Send + Sync>,
        Arc<dyn ModelCache + Send + Sync>,
        Arc<dyn ProviderStore + Send + Sync>,
        Arc<dyn TokenStore + Send + Sync>,
    ) = if let Some(pg_url) = &config.logging.pg_url {
        // Strict Postgres-only mode (no SQLite fallback)
        let pool_size = config.logging.pg_pool_size.unwrap_or(4);
        let pglog = PgLogStore::connect(pg_url, &config.logging.pg_schema, pool_size).await?;
        tracing::info!("Using PostgreSQL for logs and cache");
        let log_cache = Arc::new(pglog);
        let ts = PgTokenStore::connect(pg_url, config.logging.pg_schema.as_deref()).await?;
        (log_cache.clone(), log_cache.clone(), log_cache.clone(), Arc::new(ts))
    } else {
        let db_logger = Arc::new(DatabaseLogger::new(&config.logging.database_path).await?);
        (db_logger.clone(), db_logger.clone(), db_logger.clone(), db_logger.clone())
    };

    tracing::info!("Admin Identity Token (use as Bearer): {}", admin_identity_token);

    let app_state = AppState {
        config,
        log_store: log_store_arc,
        model_cache: model_cache_arc,
        providers: provider_store_arc,
        token_store,
        admin_identity_token,
    };

    let app = handlers::routes()
        .with_state(Arc::new(app_state));

    Ok(app)
}
