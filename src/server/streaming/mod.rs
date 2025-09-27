use axum::{extract::{Json, State}, response::{IntoResponse, Response}};
use chrono::Utc;
use std::sync::Arc;

use crate::config::settings::{KeyLogStrategy, LoggingConfig};
use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::model_redirect::apply_model_redirects;
use crate::server::provider_dispatch::select_provider_for_model;
use crate::server::AppState;

mod openai;
mod zhipu;
mod common;

pub async fn stream_chat_completions(
    State(app_state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    if !request.stream.unwrap_or(false) {
        return Err(GatewayError::Config("stream=false for streaming endpoint".into()));
    }

    let start_time = Utc::now();
    apply_model_redirects(&mut request);
    let (selected, parsed_model) = select_provider_for_model(&app_state, &request.model).await?;

    // Build upstream request with real model id
    let mut upstream_req = request.clone();
    upstream_req.model = parsed_model.get_upstream_model_name().to_string();

    match selected.provider.api_type {
        crate::config::ProviderType::OpenAI => openai::stream_openai_chat(
            app_state,
            start_time,
            request.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Zhipu => zhipu::stream_zhipu_chat(
            app_state,
            start_time,
            request.model.clone(),
            selected.provider.base_url.clone(),
            selected.provider.name.clone(),
            selected.api_key.clone(),
            upstream_req,
        )
        .await
        .map(IntoResponse::into_response),
        crate::config::ProviderType::Anthropic => {
            Err(GatewayError::Config("Anthropic streaming not implemented".into()))
        }
    }
}

pub(super) fn api_key_hint(cfg: &LoggingConfig, key: &str) -> Option<String> {
    match cfg.key_log_strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    let (start, end) = (&key[..4], &key[key.len() - 4..]);
    format!("{}****{}", start, end)
}
