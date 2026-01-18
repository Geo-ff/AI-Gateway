use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::error::GatewayError;
use crate::logging::types::{
    ProviderOpLog, REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE,
    REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST, REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET,
};
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::bearer_token;

#[derive(Debug, Serialize)]
struct ModelRedirectsOut {
    redirects: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertModelRedirectsPayload {
    redirects: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteModelRedirectPayload {
    source_model: String,
}

fn validate_redirects(redirects: &HashMap<String, String>) -> Result<(), GatewayError> {
    for (source, target) in redirects {
        if source.trim().is_empty() {
            return Err(GatewayError::Config("source_model cannot be empty".into()));
        }
        if target.trim().is_empty() {
            return Err(GatewayError::Config("target_model cannot be empty".into()));
        }
        if source.trim() == target.trim() {
            return Err(GatewayError::Config(
                "source_model cannot equal target_model".into(),
            ));
        }
    }

    // cycle detection on the directed mapping graph
    let mut visiting = HashSet::<&str>::new();
    let mut visited = HashSet::<&str>::new();

    fn dfs<'a>(
        node: &'a str,
        redirects: &'a HashMap<String, String>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> bool {
        if visited.contains(node) {
            return false;
        }
        if !visiting.insert(node) {
            return true;
        }
        if let Some(next) = redirects.get(node) {
            let next_key = next.as_str();
            if redirects.contains_key(next_key) && dfs(next_key, redirects, visiting, visited) {
                return true;
            }
        }
        visiting.remove(node);
        visited.insert(node);
        false
    }

    for node in redirects.keys() {
        if dfs(node.as_str(), redirects, &mut visiting, &mut visited) {
            return Err(GatewayError::Config(
                "redirect mapping contains a cycle".into(),
            ));
        }
    }

    Ok(())
}

pub async fn list_model_redirects(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/providers/{}/model-redirects", provider_name),
            REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let pairs = app_state
        .providers
        .list_model_redirects(&provider_name)
        .await
        .map_err(GatewayError::Db)?;
    let redirects = pairs.into_iter().collect::<HashMap<_, _>>();

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(serde_json::json!({"count": redirects.len()}).to_string()),
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        &format!("/providers/{}/model-redirects", provider_name),
        REQ_TYPE_PROVIDER_MODEL_REDIRECTS_LIST,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok(Json(ModelRedirectsOut { redirects }).into_response())
}

pub async fn replace_model_redirects(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<UpsertModelRedirectsPayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/providers/{}/model-redirects", provider_name),
            REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    validate_redirects(&payload.redirects)?;
    let now = Utc::now();
    let pairs = payload
        .redirects
        .into_iter()
        .map(|(s, t)| (s.trim().to_string(), t.trim().to_string()))
        .collect::<Vec<_>>();

    app_state
        .providers
        .replace_model_redirects(&provider_name, &pairs, now)
        .await
        .map_err(GatewayError::Db)?;

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(serde_json::json!({"count": pairs.len()}).to_string()),
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "PUT",
        &format!("/providers/{}/model-redirects", provider_name),
        REQ_TYPE_PROVIDER_MODEL_REDIRECTS_SET,
        None,
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": true })),
    )
        .into_response())
}

pub async fn delete_model_redirect(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<DeleteModelRedirectPayload>,
) -> Result<Response, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let _ = app_state
            .log_store
            .log_provider_op(ProviderOpLog {
                id: None,
                timestamp: start_time,
                operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE.to_string(),
                provider: Some(provider_name.clone()),
                details: Some(e.to_string()),
            })
            .await;
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/providers/{}/model-redirects", provider_name),
            REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE,
            None,
            Some(provider_name),
            provided_token.as_deref(),
            code,
            Some("auth failed".into()),
        )
        .await;
        return Err(e);
    }

    if !app_state
        .providers
        .provider_exists(&provider_name)
        .await
        .map_err(GatewayError::Db)?
    {
        return Err(GatewayError::NotFound(format!(
            "Provider '{}' not found",
            provider_name
        )));
    }

    let source_model = payload.source_model.trim();
    if source_model.is_empty() {
        return Err(GatewayError::Config("source_model cannot be empty".into()));
    }

    let deleted = app_state
        .providers
        .delete_model_redirect(&provider_name, source_model)
        .await
        .map_err(GatewayError::Db)?;

    let _ = app_state
        .log_store
        .log_provider_op(ProviderOpLog {
            id: None,
            timestamp: start_time,
            operation: REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE.to_string(),
            provider: Some(provider_name.clone()),
            details: Some(
                serde_json::json!({"source_model": source_model, "deleted": deleted}).to_string(),
            ),
        })
        .await;
    log_simple_request(
        &app_state,
        start_time,
        "DELETE",
        &format!("/providers/{}/model-redirects", provider_name),
        REQ_TYPE_PROVIDER_MODEL_REDIRECTS_DELETE,
        Some(source_model.to_string()),
        Some(provider_name),
        provided_token.as_deref(),
        200,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "success": deleted })),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_redirects_rejects_empty() {
        let mut m = HashMap::new();
        m.insert("".to_string(), "b".to_string());
        assert!(validate_redirects(&m).is_err());
    }

    #[test]
    fn validate_redirects_rejects_self_loop() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "a".to_string());
        assert!(validate_redirects(&m).is_err());
    }

    #[test]
    fn validate_redirects_rejects_cycle() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "b".to_string());
        m.insert("b".to_string(), "a".to_string());
        assert!(validate_redirects(&m).is_err());
    }

    #[test]
    fn validate_redirects_allows_chain() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "b".to_string());
        m.insert("b".to_string(), "c".to_string());
        assert!(validate_redirects(&m).is_ok());
    }
}
