use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use chrono::Utc;

use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, ModelListResponse, Model};
use crate::server::AppState;
use crate::server::model_helpers::fetch_provider_models;
use crate::server::model_cache::{
    get_cached_models_all,
    get_cached_models_for_provider,
    is_cache_fresh_for_all,
    is_cache_fresh_for_provider,
    cache_models_for_provider,
};
use crate::server::provider_dispatch::{select_provider, call_provider};
use crate::server::model_redirect::apply_model_redirects;
use crate::server::request_logging::log_chat_request;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route("/models/{provider}", get(list_provider_models))
}

async fn chat_completions(
    State(app_state): State<Arc<AppState>>,
    Json(mut request): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, StatusCode> {
    let start_time = Utc::now();

    apply_model_redirects(&mut request);

    let selected = select_provider(&app_state)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    let response = call_provider(&selected, &request).await;

    log_chat_request(&app_state, start_time, &request.model, &selected.provider.name, &response).await;

    response.map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// 供应商选择与调用逻辑已移动至 `server::provider_dispatch`

async fn list_models(
    State(app_state): State<Arc<AppState>>,
) -> Result<Json<ModelListResponse>, StatusCode> {
    // 尝试从缓存获取所有供应商的模型
    let cached_models = get_cached_models_all(&app_state)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !cached_models.is_empty() && is_cache_fresh_for_all(&app_state, 30).await {
        return Ok(Json(ModelListResponse {
            object: "list".to_string(),
            data: cached_models,
        }));
    }

    // 缓存过期或不存在，从所有供应商获取最新模型
    let mut all_models = Vec::new();

    for (provider_name, provider_config) in &app_state.config.providers {
        if let Some(api_key) = provider_config.api_keys.first() {
            match fetch_provider_models(provider_config, api_key).await {
                Ok(models) => {
                    // 缓存模型到数据库
                    if let Err(e) = cache_models_for_provider(&app_state, provider_name, &models).await {
                        tracing::warn!("Failed to cache models for {}: {}", provider_name, e);
                    }

                    // 添加供应商前缀到模型ID
                    for model in models {
                        all_models.push(Model {
                            id: format!("{}/{}", provider_name, model.id),
                            object: model.object,
                            created: model.created,
                            owned_by: model.owned_by,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch models from {}: {:?}", provider_name, e);
                }
            }
        }
    }

    if all_models.is_empty() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    Ok(Json(ModelListResponse {
        object: "list".to_string(),
        data: all_models,
    }))
}

async fn list_provider_models(
    Path(provider_name): Path<String>,
    State(app_state): State<Arc<AppState>>,
) -> Result<Json<ModelListResponse>, StatusCode> {
    let provider = app_state.config.providers.get(&provider_name)
        .ok_or(StatusCode::NOT_FOUND)?;

    // 先检查缓存
    if is_cache_fresh_for_provider(&app_state, &provider_name, 30).await {
        let cached_models = get_cached_models_for_provider(&app_state, &provider_name)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if !cached_models.is_empty() {
            return Ok(Json(ModelListResponse {
                object: "list".to_string(),
                data: cached_models,
            }));
        }
    }

    // 缓存过期或不存在，从供应商获取最新模型
    let api_key = provider.api_keys.first().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    match fetch_provider_models(provider, api_key).await {
        Ok(models) => {
            // 缓存新获取的模型
            if let Err(e) = cache_models_for_provider(&app_state, &provider_name, &models).await {
                tracing::warn!("Failed to cache models for {}: {}", provider_name, e);
            }

            Ok(Json(ModelListResponse {
                object: "list".to_string(),
                data: models,
            }))
        }
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

// 获取模型列表相关的辅助函数已移动到 `server::model_helpers` 模块
