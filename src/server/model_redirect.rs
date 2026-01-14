use crate::config::{ModelRedirect, Settings};
use crate::providers::openai::ChatCompletionRequest;
use crate::server::model_parser::ParsedModel;
use crate::server::AppState;
use std::collections::HashMap;
use std::collections::HashSet;

// 应用可选的模型重定向（来自 redirect.toml）
pub fn apply_model_redirects(request: &mut ChatCompletionRequest) {
    let model_redirects = Settings::load_model_redirects().unwrap_or_else(|_| ModelRedirect {
        redirects: HashMap::new(),
    });

    if let Some(redirected_model) = model_redirects.redirects.get(&request.model) {
        request.model = redirected_model.clone();
    }
}

fn resolve_redirect_chain(
    map: &HashMap<String, String>,
    source_model: &str,
    max_hops: usize,
) -> (String, bool) {
    let mut current = source_model.to_string();
    let mut seen = HashSet::<String>::new();
    for _ in 0..max_hops {
        if !seen.insert(current.clone()) {
            // cycle detected; stop at the first repeated node
            break;
        }
        match map.get(&current) {
            Some(next) if next != &current => current = next.clone(),
            _ => break,
        }
    }
    let changed = current != source_model;
    (current, changed)
}

pub async fn apply_provider_model_redirects_to_parsed_model(
    app_state: &AppState,
    provider: &str,
    parsed_model: &mut ParsedModel,
) -> Result<Option<(String, String)>, crate::error::GatewayError> {
    let pairs = app_state
        .providers
        .list_model_redirects(provider)
        .await
        .map_err(crate::error::GatewayError::Db)?;
    if pairs.is_empty() {
        return Ok(None);
    }
    let map = pairs.into_iter().collect::<HashMap<_, _>>();
    let original = parsed_model.model_name.clone();
    let (resolved, changed) = resolve_redirect_chain(&map, &original, 16);
    if !changed {
        return Ok(None);
    }
    parsed_model.model_name = resolved.clone();
    Ok(Some((original, resolved)))
}

pub async fn apply_provider_model_redirects_to_request(
    app_state: &AppState,
    provider: &str,
    request: &mut ChatCompletionRequest,
) -> Result<Option<(String, String)>, crate::error::GatewayError> {
    let mut parsed = ParsedModel::parse(&request.model);
    let applied = apply_provider_model_redirects_to_parsed_model(app_state, provider, &mut parsed)
        .await?;
    if applied.is_some() {
        request.model = if parsed.provider_name.is_some() {
            format!("{}/{}", provider, parsed.model_name)
        } else {
            parsed.model_name
        };
    }
    Ok(applied)
}
