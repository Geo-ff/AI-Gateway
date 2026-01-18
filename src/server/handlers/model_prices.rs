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
) -> Result<Json<Vec<serde_json::Value>>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let items = app_state
        .log_store
        .list_model_prices(q.provider.as_deref())
        .await
        .map_err(GatewayError::Db)?;
    let out: Vec<_> = items
        .into_iter()
        .map(|(provider, model, p_pm, c_pm, currency, model_type)| {
            serde_json::json!({
                "provider": provider,
                "model": model,
                "prompt_price_per_million": p_pm,
                "completion_price_per_million": c_pm,
                "currency": currency,
                "model_type": model_type,
            })
        })
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
