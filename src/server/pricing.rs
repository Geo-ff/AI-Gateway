use std::collections::{HashMap, HashSet};

use crate::error::GatewayError;

use super::AppState;

pub(crate) struct ResolvedModelPricing {
    pub billing_model: String,
    pub price_found: bool,
}

pub(crate) fn missing_price_allowed_for_chat(app_state: &AppState) -> bool {
    app_state
        .config
        .server
        .pricing_mode
        .allows_missing_price_for_chat()
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
    use super::resolve_redirect_chain;
    use std::collections::HashMap;

    #[test]
    fn resolve_redirect_chain_stops_on_cycle() {
        let map = HashMap::from([
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ]);

        assert_eq!(resolve_redirect_chain(&map, "a", 8), "a");
    }
}
