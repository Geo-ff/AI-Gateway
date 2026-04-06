use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::pricing::{ModelPriceView, model_price_view_from_record};
use crate::server::request_logging::log_simple_request;
use crate::server::util::bearer_token;

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
    let provided = bearer_token(&headers);
    let items = app_state
        .log_store
        .list_model_prices(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;
    let out: Vec<_> = items
        .into_iter()
        .map(model_price_view_from_record)
        .collect();
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/model-prices",
        "model_price_list_public",
        None,
        q.provider.clone(),
        provided.as_deref(),
        200,
        None,
    )
    .await;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::{ModelPriceSource, ModelPriceStatus, ModelPriceUpsert};
    use crate::server::storage_traits::RequestLogStore;
    use chrono::{Duration, Timelike, Utc};
    use tempfile::tempdir;

    #[tokio::test]
    async fn public_model_prices_list_returns_metadata_fields() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let logger = crate::logging::DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let synced_at = Utc::now().with_nanosecond(0).unwrap();

        RequestLogStore::upsert_model_price(
            &logger,
            ModelPriceUpsert {
                provider: "p1".into(),
                model: "m1".into(),
                prompt_price_per_million: 1.0,
                completion_price_per_million: 2.0,
                currency: Some("USD".into()),
                model_type: Some("chat,image".into()),
                source: ModelPriceSource::Auto,
                status: ModelPriceStatus::Stale,
                synced_at: Some(synced_at),
                expires_at: Some(synced_at + Duration::hours(1)),
            },
        )
        .await
        .unwrap();

        let state = Arc::new(crate::server::AppState {
            config: crate::config::Settings {
                load_balancing: crate::config::settings::LoadBalancing {
                    strategy: crate::config::BalanceStrategy::FirstAvailable,
                },
                server: crate::config::settings::ServerConfig::default(),
                logging: crate::config::settings::LoggingConfig {
                    database_path: db_path.to_string_lossy().to_string(),
                    ..Default::default()
                },
            },
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: Arc::new(logger.clone()),
            model_cache: Arc::new(logger.clone()),
            providers: Arc::new(logger.clone()),
            token_store: Arc::new(logger.clone()),
            favorites_store: Arc::new(logger.clone()),
            login_manager: Arc::new(crate::server::login::LoginManager::new(Arc::new(
                logger.clone(),
            ))),
            user_store: Arc::new(logger.clone()),
            refresh_token_store: Arc::new(logger.clone()),
            password_reset_token_store: Arc::new(logger.clone()),
            balance_store: Arc::new(logger.clone()),
            subscription_store: Arc::new(logger),
        });

        let Json(items) = list_model_prices(
            State(state),
            HeaderMap::new(),
            Query(ListQuery {
                provider: Some("p1".into()),
            }),
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, Some(ModelPriceSource::Auto));
        assert_eq!(items[0].status, ModelPriceStatus::Stale);
        assert_eq!(items[0].model_type.as_deref(), Some("chat"));
        assert_eq!(
            items[0].model_types,
            Some(vec!["chat".to_string(), "image".to_string()])
        );
        assert_eq!(items[0].synced_at, Some(synced_at));
    }
}
