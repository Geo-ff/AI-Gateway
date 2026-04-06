use std::collections::{HashMap, HashSet};

use crate::error::GatewayError;
use crate::logging::{ModelPriceRecord, ModelPriceSource, ModelPriceStatus};
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::AppState;
use super::model_types;

pub(crate) struct ResolvedModelPricing {
    pub billing_model: String,
    pub price_found: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ModelPriceView {
    pub provider: String,
    pub model: String,
    pub prompt_price_per_million: Option<f64>,
    pub completion_price_per_million: Option<f64>,
    pub currency: Option<String>,
    pub model_type: Option<String>,
    pub model_types: Option<Vec<String>>,
    pub source: Option<ModelPriceSource>,
    pub status: ModelPriceStatus,
    pub synced_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub(crate) fn missing_price_allowed_for_chat(app_state: &AppState) -> bool {
    app_state
        .config
        .server
        .pricing_mode
        .allows_missing_price_for_chat()
}

pub(crate) fn normalize_model_price_status(
    status: ModelPriceStatus,
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ModelPriceStatus {
    if matches!(status, ModelPriceStatus::Active)
        && let Some(expires_at) = expires_at
        && expires_at <= now
    {
        ModelPriceStatus::Stale
    } else {
        status
    }
}

pub(crate) fn normalize_model_price_record(mut record: ModelPriceRecord) -> ModelPriceRecord {
    record.status = normalize_model_price_status(record.status, record.expires_at, Utc::now());
    record
}

pub(crate) fn normalized_price_metadata(
    source: Option<ModelPriceSource>,
    status: Option<ModelPriceStatus>,
    synced_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
) -> (
    ModelPriceSource,
    ModelPriceStatus,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    let source = source.unwrap_or(ModelPriceSource::Manual);
    let status = normalize_model_price_status(
        status.unwrap_or(ModelPriceStatus::Active),
        expires_at,
        Utc::now(),
    );
    (source, status, synced_at, expires_at)
}

pub(crate) fn model_price_view_from_record(record: ModelPriceRecord) -> ModelPriceView {
    let record = normalize_model_price_record(record);
    let (model_type, model_types) =
        model_types::model_types_for_response(record.model_type.as_deref());
    ModelPriceView {
        provider: record.provider,
        model: record.model,
        prompt_price_per_million: Some(record.prompt_price_per_million),
        completion_price_per_million: Some(record.completion_price_per_million),
        currency: record.currency,
        model_type,
        model_types,
        source: Some(record.source),
        status: record.status,
        synced_at: record.synced_at,
        expires_at: record.expires_at,
    }
}

pub(crate) fn missing_model_price_view(provider: &str, model: &str) -> ModelPriceView {
    ModelPriceView {
        provider: provider.to_string(),
        model: model.to_string(),
        prompt_price_per_million: None,
        completion_price_per_million: None,
        currency: None,
        model_type: None,
        model_types: None,
        source: None,
        status: ModelPriceStatus::Missing,
        synced_at: None,
        expires_at: None,
    }
}

pub(crate) fn derive_model_price_view(
    provider: &str,
    model: &str,
    record: Option<ModelPriceRecord>,
) -> ModelPriceView {
    record
        .map(model_price_view_from_record)
        .unwrap_or_else(|| missing_model_price_view(provider, model))
}

pub(crate) async fn resolve_model_pricing(
    app_state: &AppState,
    provider_name: &str,
    upstream_model: &str,
    redirected_from_for_price: Option<&str>,
) -> Result<ResolvedModelPricing, GatewayError> {
    let mut billing_model = upstream_model.to_string();
    let mut price = app_state
        .log_store
        .get_model_price(provider_name, upstream_model)
        .await
        .map_err(GatewayError::Db)?;

    if price.is_none()
        && let Some(fallback) = redirected_from_for_price
        && let Ok(fallback_price) = app_state
            .log_store
            .get_model_price(provider_name, fallback)
            .await
        && fallback_price.is_some()
    {
        price = fallback_price;
        billing_model = fallback.to_string();
    }

    if price.is_none() {
        let pairs = app_state
            .providers
            .list_model_redirects(provider_name)
            .await
            .map_err(GatewayError::Db)?;
        if !pairs.is_empty() {
            let map: HashMap<String, String> = pairs.into_iter().collect();
            for source in map.keys() {
                if resolve_redirect_chain(&map, source, 16) != upstream_model {
                    continue;
                }
                let source_price = app_state
                    .log_store
                    .get_model_price(provider_name, source)
                    .await
                    .map_err(GatewayError::Db)?;
                if source_price.is_some() {
                    price = source_price;
                    billing_model = source.to_string();
                    break;
                }
            }
        }
    }

    Ok(ResolvedModelPricing {
        billing_model,
        price_found: price.is_some(),
    })
}

fn resolve_redirect_chain(
    map: &HashMap<String, String>,
    source_model: &str,
    max_hops: usize,
) -> String {
    let mut current = source_model.to_string();
    let mut seen = HashSet::<String>::new();
    for _ in 0..max_hops {
        if !seen.insert(current.clone()) {
            break;
        }
        match map.get(&current) {
            Some(next) if next != &current => current = next.clone(),
            _ => break,
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::{missing_model_price_view, normalize_model_price_status, resolve_redirect_chain};
    use crate::logging::ModelPriceStatus;
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    #[test]
    fn resolve_redirect_chain_stops_on_cycle() {
        let map = HashMap::from([
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ]);

        assert_eq!(resolve_redirect_chain(&map, "a", 8), "a");
    }

    #[test]
    fn normalize_model_price_status_marks_expired_active_as_stale() {
        let now = Utc::now();

        assert_eq!(
            normalize_model_price_status(
                ModelPriceStatus::Active,
                Some(now - Duration::minutes(1)),
                now,
            ),
            ModelPriceStatus::Stale
        );
    }

    #[test]
    fn missing_model_price_view_has_stable_shape() {
        let view = missing_model_price_view("p1", "m1");

        assert_eq!(view.provider, "p1");
        assert_eq!(view.model, "m1");
        assert_eq!(view.status, ModelPriceStatus::Missing);
        assert_eq!(view.source, None);
        assert_eq!(view.prompt_price_per_million, None);
    }
}
