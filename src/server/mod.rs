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
use crate::server::storage_traits::{ModelCache, RequestLogStore};
use axum::Router;
use std::sync::Arc;
use crate::error::Result as AppResult;

#[derive(Clone)]
pub struct AppState {
    pub config: Settings,
    pub log_store: Arc<dyn RequestLogStore + Send + Sync>,
    pub model_cache: Arc<dyn ModelCache + Send + Sync>,
    pub db: Arc<DatabaseLogger>,
}

pub async fn create_app(config: Settings) -> AppResult<Router> {
    let db_logger = DatabaseLogger::new(&config.logging.database_path).await?;

    let db_logger = Arc::new(db_logger);

    let app_state = AppState {
        config,
        log_store: db_logger.clone(),
        model_cache: db_logger.clone(),
        db: db_logger.clone(),
    };

    let app = handlers::routes()
        .with_state(Arc::new(app_state));

    Ok(app)
}
