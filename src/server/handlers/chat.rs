use axum::{extract::State, response::{IntoResponse, Response}, Json};
use chrono::Utc;
use std::sync::Arc;

use crate::error::GatewayError;
use crate::providers::openai::ChatCompletionRequest;
use crate::server::provider_dispatch::{call_provider_with_parsed_model, select_provider_for_model};
use crate::server::streaming::stream_chat_completions;
use crate::server::{model_redirect::apply_model_redirects, request_logging::log_chat_request, AppState};

pub async fn chat_completions(
    State(app_state): State<Arc<AppState>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Response, GatewayError> {
    if request.stream.unwrap_or(false) {
        let response = stream_chat_completions(State(app_state), Json(request)).await?;
        Ok(response.into_response())
    } else {
        let mut request = request;
        let start_time = Utc::now();
        apply_model_redirects(&mut request);

        let (selected, parsed_model) = select_provider_for_model(&app_state, &request.model).await?;
        let response = call_provider_with_parsed_model(&selected, &request, &parsed_model).await;

        // 日志使用 typed，用于提取 usage
        log_chat_request(&app_state, start_time, &request.model, &selected.provider.name, &selected.api_key, &response).await;

        // 将原始 JSON 透传给前端，以保留 reasoning_content 等扩展字段
        match response {
            Ok(dual) => Ok(Json(dual.raw).into_response()),
            Err(e) => Err(e),
        }
    }
}
