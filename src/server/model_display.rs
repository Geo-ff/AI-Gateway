use std::collections::HashMap;

use crate::config::settings::Provider;

pub fn provider_display_name(
    providers_by_id: &HashMap<String, Provider>,
    provider_id: &str,
) -> String {
    providers_by_id
        .get(provider_id)
        .and_then(|provider| provider.display_name.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(provider_id)
        .to_string()
}

pub fn format_provider_model_display_name(
    providers_by_id: &HashMap<String, Provider>,
    provider_id: &str,
    model_id: &str,
) -> String {
    format!(
        "{}/{}",
        provider_display_name(providers_by_id, provider_id),
        model_id
    )
}

pub fn format_model_display_name(
    providers_by_id: &HashMap<String, Provider>,
    model: &str,
    fallback_provider_id: Option<&str>,
) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some((provider_id, model_id)) = trimmed.split_once('/')
        && !provider_id.is_empty()
        && !model_id.is_empty()
    {
        return format_provider_model_display_name(providers_by_id, provider_id, model_id);
    }

    if let Some(provider_id) = fallback_provider_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format_provider_model_display_name(providers_by_id, provider_id, trimmed);
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{Provider, ProviderConfig, ProviderType};

    fn provider(name: &str, display_name: Option<&str>) -> Provider {
        Provider {
            name: name.to_string(),
            display_name: display_name.map(|value| value.to_string()),
            collection: "默认合集".to_string(),
            api_type: ProviderType::OpenAI,
            api_type_raw: None,
            base_url: "https://example.com".to_string(),
            api_keys: Vec::new(),
            models_endpoint: None,
            provider_config: ProviderConfig::default(),
            enabled: true,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn formats_prefixed_model_with_display_name() {
        let mut providers = HashMap::new();
        providers.insert("fox".to_string(), provider("fox", Some("Fox 渠道")));

        let value = format_model_display_name(&providers, "fox/gpt-4o", None);
        assert_eq!(value, "Fox 渠道/gpt-4o");
    }

    #[test]
    fn formats_model_with_fallback_provider() {
        let mut providers = HashMap::new();
        providers.insert("fox".to_string(), provider("fox", Some("Fox 渠道")));

        let value = format_model_display_name(&providers, "gpt-4o", Some("fox"));
        assert_eq!(value, "Fox 渠道/gpt-4o");
    }

    #[test]
    fn falls_back_to_provider_id_when_display_name_missing() {
        let mut providers = HashMap::new();
        providers.insert("fox".to_string(), provider("fox", None));

        let value = format_model_display_name(&providers, "fox/gpt-4o", None);
        assert_eq!(value, "fox/gpt-4o");
    }
}
