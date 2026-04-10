use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::admin::ClientToken;
use crate::error::GatewayError;
use crate::logging::RequestLog;
use crate::logging::types::{
    REQ_TYPE_CHAT_COMPARE, REQ_TYPE_CHAT_REPLAY, RequestLabExperimentConfig,
    RequestLogDetailRecord, StoredCompareRun, StoredRequestLabSnapshot, StoredRequestLabSource,
    StoredRequestLabTemplate,
};
use crate::providers::openai::ChatCompletionRequest;
use crate::providers::openai::types::RawAndTypedChatCompletion;
use crate::providers::openai::usage::resolved_usage;
use crate::server::AppState;
use crate::server::handlers::auth::{
    AccessTokenClaims, AdminIdentity, require_superadmin, require_user,
};
use crate::server::model_redirect::{
    apply_model_redirects, apply_provider_model_redirects_to_parsed_model,
};
use crate::server::pricing::{missing_price_allowed_for_chat, resolve_model_pricing};
use crate::server::provider_dispatch::{
    call_provider_with_parsed_model, select_provider_for_model,
};
use crate::server::request_logging::{
    ChatLogContext, LoggedChatRequest, log_chat_request, log_simple_request,
};
use crate::server::response_text;
use crate::users::UserRole;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayableRequestSnapshot {
    pub kind: String,
    pub request: serde_json::Value,
    pub top_k: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReplayOverrideInput {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<serde_json::Value>,
    #[serde(default)]
    pub top_p: Option<serde_json::Value>,
    #[serde(default)]
    pub max_tokens: Option<serde_json::Value>,
    #[serde(default)]
    pub presence_penalty: Option<serde_json::Value>,
    #[serde(default)]
    pub frequency_penalty: Option<serde_json::Value>,
    #[serde(default)]
    pub preserve_system_prompt: Option<bool>,
    #[serde(default)]
    pub preserve_message_structure: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompareRequest {
    pub source_request_id: i64,
    pub models: Vec<String>,
    #[serde(default)]
    pub temperature: Option<serde_json::Value>,
    #[serde(default)]
    pub top_p: Option<serde_json::Value>,
    #[serde(default)]
    pub max_tokens: Option<serde_json::Value>,
    #[serde(default)]
    pub presence_penalty: Option<serde_json::Value>,
    #[serde(default)]
    pub frequency_penalty: Option<serde_json::Value>,
    #[serde(default)]
    pub preserve_system_prompt: Option<bool>,
    #[serde(default)]
    pub preserve_message_structure: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRequestLabTemplateRequest {
    #[serde(default)]
    pub scope: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source_request_id: i64,
    pub compare_models: Vec<String>,
    pub experiment_config: RequestLabExperimentConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRequestLabTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub source_request_id: Option<i64>,
    #[serde(default)]
    pub compare_models: Option<Vec<String>>,
    #[serde(default)]
    pub experiment_config: Option<RequestLabExperimentConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListRequestLabTemplatesQuery {
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddRequestLabSourceRequest {
    pub source_request_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLabSourceResponse {
    pub source_request_id: i64,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub method: String,
    pub path: String,
    pub status: String,
    pub status_code: u16,
    pub source_timestamp: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequestLabSourceResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRequestLabSnapshotRequest {
    pub source_request_id: i64,
    pub compare_run_id: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRequestLabSnapshotNoteRequest {
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListRequestLabSnapshotsQuery {
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub compare_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLabSnapshotItemsSummary {
    pub success_count: u32,
    pub failure_count: u32,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalizedMessage {
    pub zh_cn: String,
    pub en: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareItemErrorInfo {
    pub code: String,
    pub i18n_key: String,
    pub message: String,
    pub localized_message: LocalizedMessage,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLabSnapshotSourcePayload {
    pub source_request_id: i64,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub method: String,
    pub path: String,
    pub status: String,
    pub status_code: u16,
    pub source_timestamp: String,
    #[serde(default)]
    pub request_payload_snapshot: Option<ReplayableRequestSnapshot>,
    #[serde(default)]
    pub response_preview: Option<String>,
    #[serde(default)]
    pub first_token_latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLabSnapshotPayload {
    pub compare: CompareResponse,
    pub source: RequestLabSnapshotSourcePayload,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLabSnapshotListItemResponse {
    pub id: String,
    pub note: Option<String>,
    pub created_at: String,
    pub source_request_id: i64,
    pub source_requested_model: Option<String>,
    pub source_effective_model: Option<String>,
    pub models: Vec<String>,
    pub items: RequestLabSnapshotItemsSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLabSnapshotDetailResponse {
    pub id: String,
    pub note: Option<String>,
    pub created_at: String,
    pub source_request_id: i64,
    pub compare_run_id: String,
    pub source_requested_model: Option<String>,
    pub source_effective_model: Option<String>,
    pub models: Vec<String>,
    pub items: RequestLabSnapshotItemsSummary,
    pub compare: CompareResponse,
    pub source: RequestLabSnapshotSourcePayload,
    pub snapshot_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareDetailResponse {
    pub id: String,
    pub source_request_id: i64,
    pub created_at: String,
    pub items: Vec<CompareItemResponse>,
    pub compare_run_id: String,
    pub source_requested_model: Option<String>,
    pub source_effective_model: Option<String>,
    pub models: Vec<String>,
    pub items_summary: RequestLabSnapshotItemsSummary,
    pub compare: CompareResponse,
    pub source: RequestLabSnapshotSourcePayload,
    pub snapshot_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequestLabSnapshotResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteRequestLabTemplateResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredRequestContent {
    #[serde(default)]
    pub text: Option<String>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredRequestMessage {
    pub role: String,
    pub content: StructuredRequestContent,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRequestSummary {
    pub source_request_id: i64,
    pub timestamp: String,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub username: Option<String>,
    pub client_token_name: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub status: String,
    pub status_code: u16,
    pub response_time_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLabTemplateResponse {
    pub id: String,
    pub scope: String,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub source_request_id: i64,
    pub compare_models: Vec<String>,
    pub experiment_config: RequestLabExperimentConfig,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareItemResponse {
    pub request_id: Option<i64>,
    pub model: String,
    pub requested_model: String,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub output_summary: Option<String>,
    pub response: Option<serde_json::Value>,
    pub response_time_ms: i64,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cost: Option<f64>,
    pub status: String,
    pub status_code: u16,
    pub error_message: Option<String>,
    #[serde(default)]
    pub error: Option<CompareItemErrorInfo>,
    #[serde(default)]
    pub upstream_status: Option<i64>,
    #[serde(default)]
    pub selected_provider: Option<String>,
    #[serde(default)]
    pub selected_key_id: Option<String>,
    #[serde(default)]
    pub first_token_latency_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareResponse {
    pub id: String,
    pub source_request_id: i64,
    pub created_at: String,
    pub items: Vec<CompareItemResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestDetailResponse {
    pub id: i64,
    pub timestamp: String,
    pub requested_model: Option<String>,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub client_token_id: Option<String>,
    pub client_token_name: Option<String>,
    pub username: Option<String>,
    pub request_type: String,
    pub path: String,
    pub method: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cost: Option<f64>,
    pub status: String,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub request_payload_snapshot: Option<serde_json::Value>,
    pub response_preview: Option<String>,
    pub upstream_status: Option<i64>,
    pub selected_provider: Option<String>,
    pub selected_key_id: Option<String>,
    pub first_token_latency_ms: Option<i64>,
    pub error_message: Option<String>,
    pub source_request_summary: SourceRequestSummary,
    #[serde(default)]
    pub system_prompt: Option<StructuredRequestContent>,
    #[serde(default)]
    pub messages: Vec<StructuredRequestMessage>,
    pub locked_fields: Vec<String>,
    pub template_applied: bool,
    #[serde(default)]
    pub template_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayResponse {
    pub source_request_id: i64,
    pub request_id: Option<i64>,
    pub requested_model: String,
    pub effective_model: Option<String>,
    pub provider: Option<String>,
    pub output_summary: Option<String>,
    pub response: Option<serde_json::Value>,
    pub response_time_ms: i64,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub cost: Option<f64>,
    pub status: String,
    pub status_code: u16,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub struct ExecutedChatRequest {
    pub effective_model: String,
    pub provider_name: String,
    pub response: Result<RawAndTypedChatCompletion, GatewayError>,
    pub upstream_error_body: Option<serde_json::Value>,
    pub logged: LoggedChatRequest,
}

fn is_superadmin(claims: &AccessTokenClaims) -> bool {
    matches!(UserRole::parse(&claims.role), Some(UserRole::Superadmin))
}

pub fn build_request_payload_snapshot(
    request: &ChatCompletionRequest,
    top_k: Option<u32>,
) -> Result<String, GatewayError> {
    let snapshot = ReplayableRequestSnapshot {
        kind: "chat_completions".to_string(),
        request: serde_json::to_value(request)?,
        top_k,
    };
    Ok(serde_json::to_string(&snapshot)?)
}

fn is_system_role(role: &str) -> bool {
    matches!(role, "system" | "developer")
}

fn message_role(message: &serde_json::Value) -> String {
    message
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn message_content_value(message: &serde_json::Value) -> serde_json::Value {
    message
        .get("content")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn collect_text_fragments(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text.clone()]
            }
        }
        serde_json::Value::Array(items) => items.iter().flat_map(collect_text_fragments).collect(),
        serde_json::Value::Object(map) => {
            for key in [
                "delta",
                "output_text",
                "text",
                "value",
                "message",
                "content",
            ] {
                if let Some(value) = map.get(key) {
                    let fragments = collect_text_fragments(value);
                    if !fragments.is_empty() {
                        return fragments;
                    }
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn join_text_fragments(fragments: Vec<String>) -> Option<String> {
    let joined = fragments.join("").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn value_display_text(value: &serde_json::Value) -> Option<String> {
    join_text_fragments(collect_text_fragments(value)).or_else(|| match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        _ => serde_json::to_string_pretty(value)
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
    })
}

fn structured_content_from_value(value: serde_json::Value) -> StructuredRequestContent {
    StructuredRequestContent {
        text: value_display_text(&value),
        raw: value,
    }
}

fn request_snapshot_parts(
    snapshot: Option<&ReplayableRequestSnapshot>,
) -> (
    Option<StructuredRequestContent>,
    Vec<StructuredRequestMessage>,
) {
    let Some(snapshot) = snapshot else {
        return (None, Vec::new());
    };
    let messages = snapshot
        .request
        .get("messages")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut system_contents = Vec::new();
    let mut system_text_parts = Vec::new();
    let mut structured_messages = Vec::new();

    for message in messages {
        let role = message_role(&message);
        let content = message_content_value(&message);
        if is_system_role(&role) {
            if let Some(text) = value_display_text(&content) {
                system_text_parts.push(text);
            }
            system_contents.push(content);
            continue;
        }

        structured_messages.push(StructuredRequestMessage {
            role,
            content: structured_content_from_value(content),
        });
    }

    let system_prompt = if system_contents.is_empty() {
        None
    } else if system_contents.len() == 1 {
        Some(StructuredRequestContent {
            text: system_text_parts
                .into_iter()
                .map(|item| item.trim().to_string())
                .find(|item| !item.is_empty()),
            raw: system_contents
                .into_iter()
                .next()
                .unwrap_or(serde_json::Value::Null),
        })
    } else {
        Some(StructuredRequestContent {
            text: {
                let joined = system_text_parts.join("\n\n").trim().to_string();
                if joined.is_empty() {
                    None
                } else {
                    Some(joined)
                }
            },
            raw: serde_json::Value::Array(system_contents),
        })
    };

    (system_prompt, structured_messages)
}

fn transformed_snapshot_messages(
    messages: Vec<serde_json::Value>,
    preserve_system_prompt: bool,
    preserve_message_structure: bool,
) -> Vec<serde_json::Value> {
    let filtered: Vec<serde_json::Value> = messages
        .into_iter()
        .filter(|message| preserve_system_prompt || !is_system_role(&message_role(message)))
        .collect();

    if preserve_message_structure {
        return filtered;
    }

    let mut retained_system_messages = Vec::new();
    let mut collapsed_blocks = Vec::new();

    for message in filtered {
        let role = message_role(&message);
        if is_system_role(&role) {
            retained_system_messages.push(message);
            continue;
        }
        let content = message_content_value(&message);
        if let Some(text) = value_display_text(&content) {
            collapsed_blocks.push(format!("{}:\n{}", role.to_uppercase(), text));
        }
    }

    if collapsed_blocks.is_empty() {
        return retained_system_messages;
    }

    retained_system_messages.push(serde_json::json!({
        "role": "user",
        "content": collapsed_blocks.join("\n\n"),
    }));
    retained_system_messages
}

fn request_summary(
    log: &RequestLog,
    client_token_name: Option<&str>,
    username: Option<&str>,
) -> SourceRequestSummary {
    SourceRequestSummary {
        source_request_id: log.id.unwrap_or_default(),
        timestamp: log.timestamp.to_rfc3339(),
        requested_model: log.requested_model.clone(),
        effective_model: log.effective_model.clone().or_else(|| log.model.clone()),
        provider: log.provider.clone(),
        username: username.map(|value| value.to_string()),
        client_token_name: client_token_name.map(|value| value.to_string()),
        input_tokens: log.prompt_tokens,
        output_tokens: log.completion_tokens,
        total_tokens: log.total_tokens,
        status: request_status(log.status_code),
        status_code: log.status_code,
        response_time_ms: log.response_time_ms,
    }
}

fn request_locked_fields() -> Vec<String> {
    vec![
        "source_request_relation".to_string(),
        "system_prompt".to_string(),
        "messages".to_string(),
        "user_input_body".to_string(),
    ]
}

fn normalize_template_scope(scope: Option<String>) -> Result<String, GatewayError> {
    let normalized = scope.unwrap_or_else(|| "personal".to_string());
    let normalized = normalized.trim().to_lowercase();
    if normalized == "personal" {
        Ok(normalized)
    } else {
        Err(GatewayError::Config("模板 scope 仅支持 personal".into()))
    }
}

fn normalize_template_name(name: String) -> Result<String, GatewayError> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(GatewayError::Config("模板名称不能为空".into()));
    }
    Ok(trimmed)
}

fn normalize_template_description(description: Option<String>) -> Option<String> {
    description.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_template_tags(tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let trimmed = tag.trim().to_string();
        if trimmed.is_empty() || normalized.iter().any(|item| item == &trimmed) {
            continue;
        }
        normalized.push(trimmed);
    }
    normalized
}

fn normalize_compare_models(models: Vec<String>) -> Result<Vec<String>, GatewayError> {
    let normalized: Vec<String> = models
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    if normalized.len() < 2 || normalized.len() > 3 {
        return Err(GatewayError::Config(
            "实验模板的模型对比方案仅支持选择 2 到 3 个模型".into(),
        ));
    }
    Ok(normalized)
}

fn template_matches_keyword(template: &StoredRequestLabTemplate, keyword: &str) -> bool {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return true;
    }

    template.name.to_lowercase().contains(&keyword)
        || template
            .description
            .as_deref()
            .map(|value| value.to_lowercase().contains(&keyword))
            .unwrap_or(false)
        || template
            .tags
            .iter()
            .any(|value| value.to_lowercase().contains(&keyword))
        || template
            .compare_models
            .iter()
            .any(|value| value.to_lowercase().contains(&keyword))
}

fn template_matches_tag(template: &StoredRequestLabTemplate, tag: &str) -> bool {
    let tag = tag.trim().to_lowercase();
    if tag.is_empty() {
        return true;
    }
    template.tags.iter().any(|item| item.to_lowercase() == tag)
}

fn request_lab_template_response(template: StoredRequestLabTemplate) -> RequestLabTemplateResponse {
    RequestLabTemplateResponse {
        id: template.id,
        scope: template.scope,
        name: template.name,
        description: template.description,
        tags: template.tags,
        source_request_id: template.source_request_id,
        compare_models: template.compare_models,
        experiment_config: template.experiment_config,
        created_by: template.created_by,
        created_at: template.created_at.to_rfc3339(),
        updated_at: template.updated_at.to_rfc3339(),
    }
}

fn snapshot_from_detail(
    detail: &RequestLogDetailRecord,
) -> Result<ReplayableRequestSnapshot, GatewayError> {
    let Some(raw) = detail.request_payload_snapshot.as_deref() else {
        return Err(GatewayError::Config(
            "请求快照缺失，当前日志不可回放".into(),
        ));
    };
    serde_json::from_str(raw)
        .map_err(|_| GatewayError::Config("请求快照已损坏，当前日志不可回放".into()))
}

fn request_from_snapshot(
    snapshot: &ReplayableRequestSnapshot,
    overrides: &ReplayOverrideInput,
) -> Result<(ChatCompletionRequest, Option<u32>), GatewayError> {
    if snapshot.kind != "chat_completions" {
        return Err(GatewayError::Config("当前请求类型暂不支持回放".into()));
    }
    let mut request = snapshot.request.clone();
    let Some(request_obj) = request.as_object_mut() else {
        return Err(GatewayError::Config("请求快照格式非法".into()));
    };
    request_obj.insert("stream".to_string(), serde_json::Value::Bool(false));
    if let Some(model) = overrides.model.as_ref() {
        request_obj.insert(
            "model".to_string(),
            serde_json::Value::String(model.clone()),
        );
    }
    if let Some(temperature) = overrides.temperature.clone() {
        request_obj.insert("temperature".to_string(), temperature);
    }
    if let Some(top_p) = overrides.top_p.clone() {
        request_obj.insert("top_p".to_string(), top_p);
    }
    if let Some(max_tokens) = overrides.max_tokens.clone() {
        request_obj.insert("max_tokens".to_string(), max_tokens);
    }
    if let Some(presence_penalty) = overrides.presence_penalty.clone() {
        request_obj.insert("presence_penalty".to_string(), presence_penalty);
    }
    if let Some(frequency_penalty) = overrides.frequency_penalty.clone() {
        request_obj.insert("frequency_penalty".to_string(), frequency_penalty);
    }
    let preserve_system_prompt = overrides.preserve_system_prompt.unwrap_or(true);
    let preserve_message_structure = overrides.preserve_message_structure.unwrap_or(true);
    if let Some(messages) = request_obj
        .get("messages")
        .and_then(|value| value.as_array())
        .cloned()
    {
        request_obj.insert(
            "messages".to_string(),
            serde_json::Value::Array(transformed_snapshot_messages(
                messages,
                preserve_system_prompt,
                preserve_message_structure,
            )),
        );
    }
    let request: ChatCompletionRequest = serde_json::from_value(request)
        .map_err(|_| GatewayError::Config("请求快照无法反序列化为可回放请求".into()))?;
    Ok((request, snapshot.top_k))
}

async fn request_owner_token(
    app_state: &AppState,
    claims: &AccessTokenClaims,
    log: &RequestLog,
) -> Result<Option<ClientToken>, GatewayError> {
    let Some(token_id) = log.client_token.as_deref() else {
        return Ok(None);
    };
    if is_superadmin(claims) {
        return app_state.token_store.get_token_by_id(token_id).await;
    }
    app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, token_id)
        .await
}

async fn load_request_log_for_user(
    app_state: &AppState,
    claims: &AccessTokenClaims,
    request_id: i64,
) -> Result<
    (
        RequestLog,
        Option<RequestLogDetailRecord>,
        Option<ClientToken>,
    ),
    GatewayError,
> {
    let log = app_state
        .log_store
        .get_request_log_by_id(request_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("请求不存在".into()))?;
    let token = request_owner_token(app_state, claims, &log).await?;
    if token.is_none() {
        return Err(GatewayError::Forbidden("无权访问该请求".into()));
    }
    let detail = app_state
        .log_store
        .get_request_log_detail(request_id)
        .await
        .map_err(GatewayError::Db)?;
    Ok((log, detail, token))
}

async fn token_name_and_username(
    app_state: &AppState,
    log: &RequestLog,
) -> Result<(Option<String>, Option<String>), GatewayError> {
    let Some(token_id) = log.client_token.as_deref() else {
        return Ok((None, None));
    };
    let token = app_state.token_store.get_token_by_id(token_id).await?;
    let token_name = token.as_ref().map(|token| token.name.clone());
    let username = if let Some(user_id) = token.and_then(|token| token.user_id) {
        app_state
            .user_store
            .get_user(&user_id)
            .await?
            .map(|user| user.username)
    } else {
        None
    };
    Ok((token_name, username))
}

fn detail_response(
    log: RequestLog,
    detail: Option<RequestLogDetailRecord>,
    client_token_name: Option<String>,
    username: Option<String>,
) -> Result<RequestDetailResponse, GatewayError> {
    let detail_snapshot = detail
        .as_ref()
        .and_then(|item| item.request_payload_snapshot.as_deref())
        .and_then(|raw| match serde_json::from_str(raw) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(
                    "failed to parse request payload snapshot for detail response: {}",
                    error
                );
                None
            }
        });
    let (system_prompt, messages) = request_snapshot_parts(detail_snapshot.as_ref());
    let source_request_summary =
        request_summary(&log, client_token_name.as_deref(), username.as_deref());
    Ok(RequestDetailResponse {
        id: log.id.unwrap_or_default(),
        timestamp: log.timestamp.to_rfc3339(),
        requested_model: log.requested_model,
        effective_model: log.effective_model,
        provider: log.provider,
        api_key: log.api_key,
        client_token_id: log.client_token,
        client_token_name,
        username,
        request_type: log.request_type,
        path: log.path,
        method: log.method,
        input_tokens: log.prompt_tokens,
        output_tokens: log.completion_tokens,
        total_tokens: log.total_tokens,
        cost: log.amount_spent,
        status: if log.status_code < 400 {
            "success".to_string()
        } else {
            "failed".to_string()
        },
        status_code: log.status_code,
        response_time_ms: log.response_time_ms,
        request_payload_snapshot: detail_snapshot.map(|snapshot: ReplayableRequestSnapshot| {
            serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null)
        }),
        response_preview: detail
            .as_ref()
            .and_then(|item| item.response_preview.clone()),
        upstream_status: detail.as_ref().and_then(|item| item.upstream_status),
        selected_provider: detail
            .as_ref()
            .and_then(|item| item.selected_provider.clone()),
        selected_key_id: detail
            .as_ref()
            .and_then(|item| item.selected_key_id.clone()),
        first_token_latency_ms: detail.as_ref().and_then(|item| item.first_token_latency_ms),
        error_message: log.error_message,
        source_request_summary,
        system_prompt,
        messages,
        locked_fields: request_locked_fields(),
        template_applied: false,
        template_name: None,
    })
}

fn ensure_request_can_be_source(
    log: &RequestLog,
    detail: Option<&RequestLogDetailRecord>,
) -> Result<(), GatewayError> {
    if !log.request_type.starts_with("chat_") {
        return Err(GatewayError::Config("当前请求类型暂不支持加入实验".into()));
    }
    let detail =
        detail.ok_or_else(|| GatewayError::Config("请求快照缺失，当前日志不可加入实验".into()))?;
    let _ = snapshot_from_detail(detail)?;
    Ok(())
}

fn stored_request_lab_source_from_log(
    user_id: String,
    log: &RequestLog,
    added_at: DateTime<Utc>,
) -> StoredRequestLabSource {
    StoredRequestLabSource {
        user_id,
        source_request_id: log.id.unwrap_or_default(),
        requested_model: log.requested_model.clone(),
        effective_model: log.effective_model.clone().or_else(|| log.model.clone()),
        provider: log.provider.clone(),
        method: log.method.clone(),
        path: log.path.clone(),
        status_code: log.status_code,
        source_timestamp: log.timestamp,
        added_at,
    }
}

fn request_lab_source_response(source: StoredRequestLabSource) -> RequestLabSourceResponse {
    RequestLabSourceResponse {
        source_request_id: source.source_request_id,
        requested_model: source.requested_model,
        effective_model: source.effective_model,
        provider: source.provider,
        method: source.method,
        path: source.path,
        status: if source.status_code < 400 {
            "success".to_string()
        } else {
            "failed".to_string()
        },
        status_code: source.status_code,
        source_timestamp: source.source_timestamp.to_rfc3339(),
        added_at: source.added_at.to_rfc3339(),
    }
}

fn request_status(status_code: u16) -> String {
    if status_code < 400 {
        "success".to_string()
    } else {
        "failed".to_string()
    }
}

fn gateway_error_detail(err: &GatewayError) -> String {
    match err {
        GatewayError::TimeParse(message)
        | GatewayError::Config(message)
        | GatewayError::NotFound(message)
        | GatewayError::RateLimited(message)
        | GatewayError::Unauthorized(message)
        | GatewayError::Forbidden(message) => message.clone(),
        _ => err.to_string(),
    }
}

fn normalize_compare_error_detail(raw: &str) -> String {
    let mut detail = raw.trim().to_string();
    loop {
        let next = if let Some(rest) = detail.strip_prefix("Config error: ") {
            Some(rest)
        } else if let Some(rest) = detail.strip_prefix("Unauthorized: ") {
            Some(rest)
        } else if let Some(rest) = detail.strip_prefix("Rate limited: ") {
            Some(rest)
        } else if let Some(rest) = detail.strip_prefix("Not found: ") {
            Some(rest)
        } else if let Some(rest) = detail.strip_prefix("Forbidden: ") {
            Some(rest)
        } else {
            None
        };
        let Some(next) = next else {
            break;
        };
        detail = next.trim().to_string();
    }
    detail
}

fn compare_item_error_info(err: &GatewayError) -> CompareItemErrorInfo {
    let detail = normalize_compare_error_detail(&gateway_error_detail(err));
    let detail_lc = detail.to_lowercase();

    let (code, i18n_key, zh_cn, en) = if detail_lc.contains("insufficient account balance")
        || detail.contains("余额不足")
    {
        (
            "insufficient_balance",
            "request_lab.compare.error.insufficient_balance",
            "余额不足，请充值后重试。若当前令牌已被停用，请在充值或订阅后手动启用。",
            "Insufficient balance. Please top up and try again. If the token was disabled, re-enable it after topping up or renewing your subscription.",
        )
    } else if detail_lc.contains("token budget exceeded") {
        (
            "token_budget_exceeded",
            "request_lab.compare.error.token_budget_exceeded",
            "当前令牌预算已用尽，无法继续发起对比请求。",
            "This token has exhausted its budget and cannot be used for comparison requests.",
        )
    } else if detail_lc.contains("token disabled") {
        (
            "token_disabled",
            "request_lab.compare.error.token_disabled",
            "当前令牌已被停用，请启用后再试。",
            "This token is disabled. Enable it before trying again.",
        )
    } else if detail_lc.contains("token expired") {
        (
            "token_expired",
            "request_lab.compare.error.token_expired",
            "当前令牌已过期，请更换可用令牌后重试。",
            "This token has expired. Switch to an active token and try again.",
        )
    } else if detail_lc.contains("token total usage exceeded") {
        (
            "token_usage_exceeded",
            "request_lab.compare.error.token_usage_exceeded",
            "当前令牌的总用量已达到上限，无法继续发起对比请求。",
            "This token has reached its total usage limit and cannot be used for comparison requests.",
        )
    } else if detail_lc.contains("model is disabled") {
        (
            "model_disabled",
            "request_lab.compare.error.model_disabled",
            "该模型当前已被禁用，请选择其他模型。",
            "This model is currently disabled. Please choose another model.",
        )
    } else if detail_lc.contains("model price not set") {
        (
            "model_price_not_set",
            "request_lab.compare.error.model_price_not_set",
            "该模型尚未配置价格信息，暂时无法参与对比。",
            "Pricing is not configured for this model, so it cannot be used in comparisons yet.",
        )
    } else if detail_lc.contains("invalid token") || detail.contains("缺少可用令牌") {
        (
            "token_unavailable",
            "request_lab.compare.error.token_unavailable",
            "当前请求缺少可用令牌，无法完成模型对比。",
            "No usable token is available for this request, so the comparison cannot be completed.",
        )
    } else if detail_lc.contains("rate limit") {
        (
            "rate_limited",
            "request_lab.compare.error.rate_limited",
            "请求过于频繁，请稍后重试。",
            "Too many requests. Please try again later.",
        )
    } else if detail_lc.contains("authentication") || matches!(err, GatewayError::Unauthorized(_)) {
        (
            "authentication_failed",
            "request_lab.compare.error.authentication_failed",
            "模型服务鉴权失败，请检查凭证配置后重试。",
            "Authentication with the model provider failed. Please verify the credentials and try again.",
        )
    } else if detail_lc.contains("upstream returned error payload") {
        (
            "upstream_error",
            "request_lab.compare.error.upstream_error",
            "模型服务返回了错误响应，本次对比未成功。",
            "The model provider returned an error response, so this comparison attempt failed.",
        )
    } else {
        (
            "request_failed",
            "request_lab.compare.error.request_failed",
            "本次对比请求失败，请稍后重试或检查模型与令牌配置。",
            "This comparison request failed. Please try again later or review the model and token configuration.",
        )
    };

    CompareItemErrorInfo {
        code: code.to_string(),
        i18n_key: i18n_key.to_string(),
        message: zh_cn.to_string(),
        localized_message: LocalizedMessage {
            zh_cn: zh_cn.to_string(),
            en: en.to_string(),
        },
        detail: if detail.is_empty() {
            None
        } else {
            Some(detail)
        },
    }
}

fn failed_compare_item(
    request_id: Option<i64>,
    model: String,
    requested_model: String,
    effective_model: Option<String>,
    provider: Option<String>,
    response_time_ms: i64,
    cost: Option<f64>,
    upstream_status: Option<i64>,
    selected_provider: Option<String>,
    selected_key_id: Option<String>,
    first_token_latency_ms: Option<i64>,
    err: &GatewayError,
) -> CompareItemResponse {
    let error = compare_item_error_info(err);
    CompareItemResponse {
        request_id,
        model,
        requested_model,
        effective_model,
        provider,
        output_summary: None,
        response: None,
        response_time_ms,
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        cost,
        status: "failed".to_string(),
        status_code: err.status_code().as_u16(),
        error_message: Some(error.message.clone()),
        error: Some(error),
        upstream_status,
        selected_provider,
        selected_key_id,
        first_token_latency_ms,
    }
}

fn normalize_snapshot_note(note: Option<String>) -> Option<String> {
    note.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn snapshot_items_summary(items: &[CompareItemResponse]) -> RequestLabSnapshotItemsSummary {
    let success_count = items.iter().filter(|item| item.status == "success").count() as u32;
    let failure_count = items.len() as u32 - success_count;
    RequestLabSnapshotItemsSummary {
        success_count,
        failure_count,
        total_count: items.len() as u32,
    }
}

fn request_lab_snapshot_source_payload(log: &RequestLog) -> RequestLabSnapshotSourcePayload {
    RequestLabSnapshotSourcePayload {
        source_request_id: log.id.unwrap_or_default(),
        requested_model: log.requested_model.clone(),
        effective_model: log.effective_model.clone().or_else(|| log.model.clone()),
        provider: log.provider.clone(),
        method: log.method.clone(),
        path: log.path.clone(),
        status: request_status(log.status_code),
        status_code: log.status_code,
        source_timestamp: log.timestamp.to_rfc3339(),
        request_payload_snapshot: None,
        response_preview: None,
        first_token_latency_ms: None,
    }
}

fn request_lab_snapshot_source_payload_with_detail(
    log: &RequestLog,
    detail: Option<&RequestLogDetailRecord>,
) -> Result<RequestLabSnapshotSourcePayload, GatewayError> {
    let mut payload = request_lab_snapshot_source_payload(log);
    payload.request_payload_snapshot = detail
        .and_then(|item| item.request_payload_snapshot.as_deref())
        .and_then(|raw| match serde_json::from_str(raw) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(
                    "failed to parse request payload snapshot for snapshot payload: {}",
                    error
                );
                None
            }
        });
    payload.response_preview = detail.and_then(|item| item.response_preview.clone());
    payload.first_token_latency_ms = detail.and_then(|item| item.first_token_latency_ms);
    Ok(payload)
}

fn request_lab_snapshot_list_item_response(
    snapshot: &StoredRequestLabSnapshot,
) -> RequestLabSnapshotListItemResponse {
    RequestLabSnapshotListItemResponse {
        id: snapshot.id.clone(),
        note: snapshot.note.clone(),
        created_at: snapshot.created_at.to_rfc3339(),
        source_request_id: snapshot.source_request_id,
        source_requested_model: snapshot.source_requested_model.clone(),
        source_effective_model: snapshot.source_effective_model.clone(),
        models: snapshot.models.clone(),
        items: RequestLabSnapshotItemsSummary {
            success_count: snapshot.success_count,
            failure_count: snapshot.failure_count,
            total_count: snapshot.success_count + snapshot.failure_count,
        },
    }
}

fn request_lab_snapshot_detail_response(
    snapshot: StoredRequestLabSnapshot,
) -> Result<RequestLabSnapshotDetailResponse, GatewayError> {
    let snapshot_json: serde_json::Value = serde_json::from_str(&snapshot.snapshot_json)
        .map_err(|_| GatewayError::Config("历史快照已损坏".into()))?;
    let payload: RequestLabSnapshotPayload = serde_json::from_value(snapshot_json.clone())
        .map_err(|_| GatewayError::Config("历史快照详情格式非法".into()))?;
    Ok(RequestLabSnapshotDetailResponse {
        id: snapshot.id,
        note: snapshot.note,
        created_at: snapshot.created_at.to_rfc3339(),
        source_request_id: snapshot.source_request_id,
        compare_run_id: snapshot.compare_run_id,
        source_requested_model: snapshot.source_requested_model,
        source_effective_model: snapshot.source_effective_model,
        models: snapshot.models,
        items: RequestLabSnapshotItemsSummary {
            success_count: snapshot.success_count,
            failure_count: snapshot.failure_count,
            total_count: snapshot.success_count + snapshot.failure_count,
        },
        compare: payload.compare,
        source: payload.source,
        snapshot_json,
    })
}

fn compare_detail_response(
    compare: CompareResponse,
    log: &RequestLog,
    detail: Option<&RequestLogDetailRecord>,
) -> Result<CompareDetailResponse, GatewayError> {
    let source = request_lab_snapshot_source_payload_with_detail(log, detail)?;
    let snapshot_json = serde_json::to_value(RequestLabSnapshotPayload {
        compare: compare.clone(),
        source: source.clone(),
    })
    .map_err(|_| GatewayError::Config("对比结果格式非法".into()))?;
    let items_summary = snapshot_items_summary(&compare.items);
    Ok(CompareDetailResponse {
        id: compare.id.clone(),
        source_request_id: compare.source_request_id,
        created_at: compare.created_at.clone(),
        items: compare.items.clone(),
        compare_run_id: compare.id.clone(),
        source_requested_model: source.requested_model.clone(),
        source_effective_model: source.effective_model.clone(),
        models: compare
            .items
            .iter()
            .map(|item| item.model.clone())
            .collect(),
        items_summary,
        compare,
        source,
        snapshot_json,
    })
}

async fn load_compare_item_detail(
    app_state: &Arc<AppState>,
    request_id: Option<i64>,
) -> Result<Option<RequestLogDetailRecord>, GatewayError> {
    let Some(request_id) = request_id else {
        return Ok(None);
    };
    app_state
        .log_store
        .get_request_log_detail(request_id)
        .await
        .map_err(GatewayError::Db)
}

async fn compare_item_response_from_execution(
    app_state: &Arc<AppState>,
    requested_model: String,
    executed: ExecutedChatRequest,
) -> Result<CompareItemResponse, GatewayError> {
    let detail = load_compare_item_detail(app_state, executed.logged.log_id).await?;
    let upstream_status = detail.as_ref().and_then(|item| item.upstream_status);
    let selected_provider = detail
        .as_ref()
        .and_then(|item| item.selected_provider.clone())
        .or_else(|| Some(executed.provider_name.clone()));
    let selected_key_id = detail
        .as_ref()
        .and_then(|item| item.selected_key_id.clone());
    let first_token_latency_ms = detail.as_ref().and_then(|item| item.first_token_latency_ms);

    Ok(match &executed.response {
        Ok(dual) => {
            let usage = resolved_usage(&dual.raw, &dual.typed);
            CompareItemResponse {
                request_id: executed.logged.log_id,
                model: requested_model.clone(),
                requested_model,
                effective_model: Some(executed.effective_model.clone()),
                provider: Some(executed.provider_name.clone()),
                output_summary: response_text::response_summary(dual, 1200),
                response: Some(dual.raw.clone()),
                response_time_ms: executed.logged.response_time_ms,
                input_tokens: usage.as_ref().map(|usage| usage.prompt_tokens),
                output_tokens: usage.as_ref().map(|usage| usage.completion_tokens),
                total_tokens: usage.as_ref().map(|usage| usage.total_tokens),
                cost: executed.logged.amount_spent,
                status: "success".to_string(),
                status_code: 200,
                error_message: None,
                error: None,
                upstream_status,
                selected_provider,
                selected_key_id,
                first_token_latency_ms,
            }
        }
        Err(err) => failed_compare_item(
            executed.logged.log_id,
            requested_model.clone(),
            requested_model,
            Some(executed.effective_model.clone()),
            Some(executed.provider_name.clone()),
            executed.logged.response_time_ms,
            executed.logged.amount_spent,
            upstream_status,
            selected_provider,
            selected_key_id,
            first_token_latency_ms,
            err,
        ),
    })
}

fn snapshot_matches_keyword(snapshot: &StoredRequestLabSnapshot, keyword: &str) -> bool {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return true;
    }

    snapshot
        .note
        .as_deref()
        .map(|value| value.to_lowercase().contains(&keyword))
        .unwrap_or(false)
        || snapshot
            .source_requested_model
            .as_deref()
            .map(|value| value.to_lowercase().contains(&keyword))
            .unwrap_or(false)
        || snapshot
            .source_effective_model
            .as_deref()
            .map(|value| value.to_lowercase().contains(&keyword))
            .unwrap_or(false)
        || snapshot
            .models
            .iter()
            .any(|value| value.to_lowercase().contains(&keyword))
}

pub async fn execute_logged_chat_request(
    app_state: &Arc<AppState>,
    start_time: DateTime<Utc>,
    mut request: ChatCompletionRequest,
    top_k: Option<u32>,
    raw_client_token: &str,
    path: &str,
    request_type: &str,
    request_payload_snapshot: Option<String>,
) -> Result<ExecutedChatRequest, GatewayError> {
    let requested_model = request.model.clone();
    apply_model_redirects(&mut request);
    let parsed_for_prefix = crate::server::model_parser::ParsedModel::parse(&request.model);
    if let Some(provider_name) = parsed_for_prefix.provider_name.as_deref() {
        let mut parsed = parsed_for_prefix.clone();
        if let Some((from, to)) =
            apply_provider_model_redirects_to_parsed_model(app_state, provider_name, &mut parsed)
                .await?
        {
            return Err(GatewayError::Config(format!(
                "model '{}' is redirected; use '{}' instead",
                from, to
            )));
        }
    }

    let token = app_state
        .token_store
        .get_token(raw_client_token)
        .await?
        .ok_or_else(|| GatewayError::Config("invalid token".into()))?;

    if let Some(user_id) = token.user_id.as_deref() {
        let user = app_state.user_store.get_user(user_id).await?;
        let balance = user.as_ref().map(|item| item.balance).unwrap_or(0.0);
        if balance <= 0.0 {
            let _ = app_state
                .token_store
                .set_enabled_for_user(user_id, false)
                .await;
            return Err(GatewayError::Config(
                "余额不足：密钥已失效；充值/订阅后需手动启用密钥".into(),
            ));
        }
    }

    if !token.enabled {
        if let Some(max_amount) = token.max_amount
            && let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&token.id)
                .await
            && spent >= max_amount
        {
            return Err(GatewayError::Config("token budget exceeded".into()));
        }
        return Err(GatewayError::Config("token disabled".into()));
    }

    if let Some(expires_at) = token.expires_at
        && Utc::now() > expires_at
    {
        return Err(GatewayError::Config("token expired".into()));
    }

    if let Some(max_tokens) = token.max_tokens
        && token.total_tokens_spent >= max_tokens
    {
        return Err(GatewayError::Config("token total usage exceeded".into()));
    }

    let (selected, parsed_model) = select_provider_for_model(app_state, &request.model).await?;
    let upstream_model = parsed_model.get_upstream_model_name().to_string();

    if let Ok(Some(false)) = app_state
        .log_store
        .get_model_enabled(&selected.provider.name, &upstream_model)
        .await
    {
        return Err(GatewayError::Config("model is disabled".into()));
    }

    let resolved_pricing =
        resolve_model_pricing(app_state, &selected.provider.name, &upstream_model, None).await?;
    if !resolved_pricing.price_found && !missing_price_allowed_for_chat(app_state) {
        return Err(GatewayError::Config("model price not set".into()));
    }

    let response = call_provider_with_parsed_model(&selected, &request, &parsed_model, top_k).await;
    let upstream_error_body = response
        .as_ref()
        .ok()
        .filter(|dual| dual.raw.get("error").is_some() && dual.raw.get("choices").is_none())
        .map(|dual| dual.raw.clone());

    let response_for_log: Result<RawAndTypedChatCompletion, GatewayError> =
        if let Some(body) = upstream_error_body.as_ref() {
            Err(GatewayError::Config(format!(
                "upstream returned error payload: {}",
                body
            )))
        } else {
            match &response {
                Ok(dual) => Ok(dual.clone()),
                Err(err) => Err(GatewayError::Config(err.to_string())),
            }
        };
    let logged = log_chat_request(
        app_state,
        start_time,
        &resolved_pricing.billing_model,
        &requested_model,
        &upstream_model,
        &selected.provider.name,
        &selected.api_key,
        Some(raw_client_token),
        &response_for_log,
        ChatLogContext {
            path: path.to_string(),
            request_type: request_type.to_string(),
            request_payload_snapshot,
            upstream_status: Some(if response.is_ok() { 200 } else { 500 }),
            selected_provider: Some(selected.provider.name.clone()),
            selected_key_id: Some(crate::server::util::mask_key(&selected.api_key)),
            first_token_latency_ms: None,
        },
    )
    .await;

    if let Ok(Some(updated)) = app_state.token_store.get_token(raw_client_token).await {
        if let Some(max_amount) = updated.max_amount
            && updated.amount_spent > max_amount
        {
            let _ = app_state
                .token_store
                .set_enabled(raw_client_token, false)
                .await;
        }
        if let Some(max_tokens) = updated.max_tokens
            && updated.total_tokens_spent > max_tokens
        {
            let _ = app_state
                .token_store
                .set_enabled(raw_client_token, false)
                .await;
        }
    }

    Ok(ExecutedChatRequest {
        effective_model: upstream_model,
        provider_name: selected.provider.name,
        response,
        upstream_error_body,
        logged,
    })
}

fn replay_response(
    source_request_id: i64,
    requested_model: String,
    result: &ExecutedChatRequest,
) -> ReplayResponse {
    match &result.response {
        Ok(dual) => {
            let usage = resolved_usage(&dual.raw, &dual.typed);
            ReplayResponse {
                source_request_id,
                request_id: result.logged.log_id,
                requested_model,
                effective_model: Some(result.effective_model.clone()),
                provider: Some(result.provider_name.clone()),
                output_summary: response_text::response_summary(dual, 1200),
                response: Some(dual.raw.clone()),
                response_time_ms: result.logged.response_time_ms,
                input_tokens: usage.as_ref().map(|usage| usage.prompt_tokens),
                output_tokens: usage.as_ref().map(|usage| usage.completion_tokens),
                total_tokens: usage.as_ref().map(|usage| usage.total_tokens),
                cost: result.logged.amount_spent,
                status: "success".to_string(),
                status_code: 200,
                error_message: None,
            }
        }
        Err(err) => ReplayResponse {
            source_request_id,
            request_id: result.logged.log_id,
            requested_model,
            effective_model: Some(result.effective_model.clone()),
            provider: Some(result.provider_name.clone()),
            output_summary: None,
            response: None,
            response_time_ms: result.logged.response_time_ms,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost: result.logged.amount_spent,
            status: "failed".to_string(),
            status_code: err.status_code().as_u16(),
            error_message: Some(err.to_string()),
        },
    }
}

pub async fn get_my_request_detail(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<i64>,
) -> Result<Json<RequestDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let (log, detail, _) = load_request_log_for_user(&app_state, &claims, request_id).await?;
    let (token_name, username) = token_name_and_username(&app_state, &log).await?;
    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        &format!("/me/requests/{request_id}"),
        "me_request_detail",
        None,
        None,
        Some("jwt"),
        200,
        None,
    )
    .await;
    Ok(Json(detail_response(log, detail, token_name, username)?))
}

pub async fn get_admin_request_detail(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<i64>,
) -> Result<Json<RequestDetailResponse>, GatewayError> {
    let identity = require_superadmin(&headers, &app_state).await?;
    let log = app_state
        .log_store
        .get_request_log_by_id(request_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("请求不存在".into()))?;
    let detail = app_state
        .log_store
        .get_request_log_detail(request_id)
        .await
        .map_err(GatewayError::Db)?;
    let (token_name, username) = token_name_and_username(&app_state, &log).await?;
    let client_name = match identity {
        AdminIdentity::Jwt(_) => "jwt",
        AdminIdentity::TuiSession(_) => "tui_session",
        AdminIdentity::WebSession(_) => "web_session",
    };
    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        &format!("/admin/requests/{request_id}"),
        "admin_request_detail",
        None,
        None,
        Some(client_name),
        200,
        None,
    )
    .await;
    Ok(Json(detail_response(log, detail, token_name, username)?))
}

pub async fn replay_my_request(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<i64>,
    Json(overrides): Json<ReplayOverrideInput>,
) -> Result<Json<ReplayResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let (log, detail, token) = load_request_log_for_user(&app_state, &claims, request_id).await?;
    if !log.request_type.starts_with("chat_") {
        return Err(GatewayError::Config("当前请求类型暂不支持原样回放".into()));
    }
    let detail =
        detail.ok_or_else(|| GatewayError::Config("请求快照缺失，当前日志不可回放".into()))?;
    let token =
        token.ok_or_else(|| GatewayError::Config("当前请求缺少可用令牌，无法回放".into()))?;
    let snapshot = snapshot_from_detail(&detail)?;
    let (request, top_k) = request_from_snapshot(&snapshot, &overrides)?;
    let requested_model = request.model.clone();
    let snapshot_json = build_request_payload_snapshot(&request, top_k)?;
    let result = execute_logged_chat_request(
        &app_state,
        Utc::now(),
        request,
        top_k,
        &token.token,
        &format!("/me/requests/{request_id}/replay"),
        REQ_TYPE_CHAT_REPLAY,
        Some(snapshot_json),
    )
    .await?;
    Ok(Json(replay_response(request_id, requested_model, &result)))
}

pub async fn add_request_lab_source(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<AddRequestLabSourceRequest>,
) -> Result<Json<RequestLabSourceResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let (log, detail, _) =
        load_request_log_for_user(&app_state, &claims, payload.source_request_id).await?;
    ensure_request_can_be_source(&log, detail.as_ref())?;

    let stored = app_state
        .log_store
        .upsert_request_lab_source(stored_request_lab_source_from_log(
            claims.sub,
            &log,
            Utc::now(),
        ))
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(request_lab_source_response(stored)))
}

pub async fn list_request_lab_sources(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<RequestLabSourceResponse>>, GatewayError> {
    let claims = require_user(&headers)?;
    let items = app_state
        .log_store
        .list_request_lab_sources(&claims.sub)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(
        items.into_iter().map(request_lab_source_response).collect(),
    ))
}

pub async fn delete_request_lab_source(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(source_request_id): Path<i64>,
) -> Result<Json<DeleteRequestLabSourceResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let deleted = app_state
        .log_store
        .delete_request_lab_source(&claims.sub, source_request_id)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(DeleteRequestLabSourceResponse { deleted }))
}

pub async fn create_request_lab_snapshot(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateRequestLabSnapshotRequest>,
) -> Result<Json<RequestLabSnapshotDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let compare_run = app_state
        .log_store
        .get_compare_run(&payload.compare_run_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("对比记录不存在".into()))?;
    if compare_run.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权保存该对比结果".into()));
    }
    if compare_run.source_request_id != payload.source_request_id {
        return Err(GatewayError::Config("快照来源请求与对比结果不匹配".into()));
    }

    let compare: CompareResponse = serde_json::from_str(&compare_run.result_json)
        .map_err(|_| GatewayError::Config("对比记录已损坏".into()))?;
    let (log, detail, _) =
        load_request_log_for_user(&app_state, &claims, payload.source_request_id).await?;
    ensure_request_can_be_source(&log, detail.as_ref())?;

    let created_at = Utc::now();
    let items_summary = snapshot_items_summary(&compare.items);
    let source_payload = request_lab_snapshot_source_payload_with_detail(&log, detail.as_ref())?;
    let snapshot_payload = RequestLabSnapshotPayload {
        compare,
        source: source_payload,
    };
    let existing = app_state
        .log_store
        .get_request_lab_snapshot_by_compare_run(&claims.sub, &payload.compare_run_id)
        .await
        .map_err(GatewayError::Db)?;
    let stored = StoredRequestLabSnapshot {
        id: existing
            .as_ref()
            .map(|snapshot| snapshot.id.clone())
            .unwrap_or_else(|| format!("snap_{}", Uuid::new_v4().simple())),
        user_id: claims.sub,
        source_request_id: payload.source_request_id,
        compare_run_id: payload.compare_run_id,
        note: normalize_snapshot_note(payload.note),
        created_at: existing
            .as_ref()
            .map(|snapshot| snapshot.created_at)
            .unwrap_or(created_at),
        snapshot_json: serde_json::to_string(&snapshot_payload)?,
        source_requested_model: log.requested_model.clone(),
        source_effective_model: log.effective_model.clone().or_else(|| log.model.clone()),
        models: snapshot_payload
            .compare
            .items
            .iter()
            .map(|item| item.model.clone())
            .collect(),
        success_count: items_summary.success_count,
        failure_count: items_summary.failure_count,
    };

    app_state
        .log_store
        .save_request_lab_snapshot(stored.clone())
        .await
        .map_err(GatewayError::Db)?;
    let persisted = app_state
        .log_store
        .get_request_lab_snapshot_by_compare_run(&stored.user_id, &stored.compare_run_id)
        .await
        .map_err(GatewayError::Db)?
        .unwrap_or(stored);
    Ok(Json(request_lab_snapshot_detail_response(persisted)?))
}

pub async fn list_request_lab_snapshots(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListRequestLabSnapshotsQuery>,
) -> Result<Json<Vec<RequestLabSnapshotListItemResponse>>, GatewayError> {
    let claims = require_user(&headers)?;
    let mut snapshots = app_state
        .log_store
        .list_request_lab_snapshots(&claims.sub)
        .await
        .map_err(GatewayError::Db)?;

    if let Some(compare_run_id) = query.compare_run_id.as_deref() {
        snapshots.retain(|snapshot| snapshot.compare_run_id == compare_run_id);
    }

    if let Some(keyword) = query.keyword.as_deref() {
        snapshots.retain(|snapshot| snapshot_matches_keyword(snapshot, keyword));
    }

    let sort_order = query.sort.as_deref().unwrap_or("desc");
    snapshots.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then(left.id.cmp(&right.id))
    });
    if sort_order != "asc" {
        snapshots.reverse();
    }

    Ok(Json(
        snapshots
            .iter()
            .map(request_lab_snapshot_list_item_response)
            .collect(),
    ))
}

pub async fn get_request_lab_snapshot(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(snapshot_id): Path<String>,
) -> Result<Json<RequestLabSnapshotDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let snapshot = app_state
        .log_store
        .get_request_lab_snapshot(&snapshot_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("历史快照不存在".into()))?;
    if snapshot.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权访问该历史快照".into()));
    }
    Ok(Json(request_lab_snapshot_detail_response(snapshot)?))
}

pub async fn update_request_lab_snapshot_note(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(snapshot_id): Path<String>,
    Json(payload): Json<UpdateRequestLabSnapshotNoteRequest>,
) -> Result<Json<RequestLabSnapshotDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let snapshot = app_state
        .log_store
        .get_request_lab_snapshot(&snapshot_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("历史快照不存在".into()))?;
    if snapshot.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权访问该历史快照".into()));
    }

    let updated = app_state
        .log_store
        .update_request_lab_snapshot_note(&snapshot_id, normalize_snapshot_note(payload.note))
        .await
        .map_err(GatewayError::Db)?;
    if !updated {
        return Err(GatewayError::NotFound("历史快照不存在".into()));
    }

    let snapshot = app_state
        .log_store
        .get_request_lab_snapshot(&snapshot_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("历史快照不存在".into()))?;
    Ok(Json(request_lab_snapshot_detail_response(snapshot)?))
}

pub async fn delete_request_lab_snapshot(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(snapshot_id): Path<String>,
) -> Result<Json<DeleteRequestLabSnapshotResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let deleted = app_state
        .log_store
        .delete_request_lab_snapshot(&claims.sub, &snapshot_id)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(DeleteRequestLabSnapshotResponse { deleted }))
}

pub async fn list_request_lab_templates(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListRequestLabTemplatesQuery>,
) -> Result<Json<Vec<RequestLabTemplateResponse>>, GatewayError> {
    let claims = require_user(&headers)?;
    let mut templates = app_state
        .log_store
        .list_request_lab_templates(&claims.sub)
        .await
        .map_err(GatewayError::Db)?;

    if let Some(keyword) = query.keyword.as_deref() {
        templates.retain(|template| template_matches_keyword(template, keyword));
    }
    if let Some(tag) = query.tag.as_deref() {
        templates.retain(|template| template_matches_tag(template, tag));
    }

    let sort_order = query.sort.as_deref().unwrap_or("desc");
    templates.sort_by(|left, right| {
        left.updated_at
            .cmp(&right.updated_at)
            .then(left.id.cmp(&right.id))
    });
    if sort_order != "asc" {
        templates.reverse();
    }

    Ok(Json(
        templates
            .into_iter()
            .map(request_lab_template_response)
            .collect(),
    ))
}

pub async fn create_request_lab_template(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateRequestLabTemplateRequest>,
) -> Result<(axum::http::StatusCode, Json<RequestLabTemplateResponse>), GatewayError> {
    let claims = require_user(&headers)?;
    let (log, detail, _) =
        load_request_log_for_user(&app_state, &claims, payload.source_request_id).await?;
    ensure_request_can_be_source(&log, detail.as_ref())?;

    let now = Utc::now();
    let stored = StoredRequestLabTemplate {
        id: format!("tmpl_{}", Uuid::new_v4().simple()),
        user_id: claims.sub.clone(),
        scope: normalize_template_scope(payload.scope)?,
        name: normalize_template_name(payload.name)?,
        description: normalize_template_description(payload.description),
        tags: normalize_template_tags(payload.tags),
        source_request_id: payload.source_request_id,
        compare_models: normalize_compare_models(payload.compare_models)?,
        experiment_config: payload.experiment_config,
        created_by: claims.sub,
        created_at: now,
        updated_at: now,
    };

    app_state
        .log_store
        .save_request_lab_template(stored.clone())
        .await
        .map_err(GatewayError::Db)?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(request_lab_template_response(stored)),
    ))
}

pub async fn get_request_lab_template(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(template_id): Path<String>,
) -> Result<Json<RequestLabTemplateResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let template = app_state
        .log_store
        .get_request_lab_template(&template_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("实验模板不存在".into()))?;
    if template.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权访问该实验模板".into()));
    }
    Ok(Json(request_lab_template_response(template)))
}

pub async fn update_request_lab_template(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(template_id): Path<String>,
    Json(payload): Json<UpdateRequestLabTemplateRequest>,
) -> Result<Json<RequestLabTemplateResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let existing = app_state
        .log_store
        .get_request_lab_template(&template_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("实验模板不存在".into()))?;
    if existing.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权修改该实验模板".into()));
    }

    let metadata_changed =
        payload.name.is_some() || payload.description.is_some() || payload.tags.is_some();
    let method_changed = payload.compare_models.is_some()
        || payload.experiment_config.is_some()
        || payload.source_request_id.is_some();
    if !metadata_changed && !method_changed {
        return Err(GatewayError::Config("没有可更新的模板内容".into()));
    }

    let mut updated = existing.clone();
    if let Some(name) = payload.name {
        updated.name = normalize_template_name(name)?;
    }
    if let Some(description) = payload.description {
        updated.description = normalize_template_description(Some(description));
    }
    if let Some(tags) = payload.tags {
        updated.tags = normalize_template_tags(tags);
    }
    if let Some(source_request_id) = payload.source_request_id {
        let (log, detail, _) =
            load_request_log_for_user(&app_state, &claims, source_request_id).await?;
        ensure_request_can_be_source(&log, detail.as_ref())?;
        updated.source_request_id = source_request_id;
    }
    if let Some(compare_models) = payload.compare_models {
        updated.compare_models = normalize_compare_models(compare_models)?;
    }
    if let Some(experiment_config) = payload.experiment_config {
        updated.experiment_config = experiment_config;
    }
    updated.updated_at = Utc::now();

    app_state
        .log_store
        .save_request_lab_template(updated.clone())
        .await
        .map_err(GatewayError::Db)?;

    Ok(Json(request_lab_template_response(updated)))
}

pub async fn delete_request_lab_template(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(template_id): Path<String>,
) -> Result<Json<DeleteRequestLabTemplateResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let deleted = app_state
        .log_store
        .delete_request_lab_template(&claims.sub, &template_id)
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(DeleteRequestLabTemplateResponse { deleted }))
}

pub async fn create_compare(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CompareRequest>,
) -> Result<Json<CompareDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    if payload.models.len() < 2 || payload.models.len() > 3 {
        return Err(GatewayError::Config(
            "模型对比仅支持选择 2 到 3 个模型".into(),
        ));
    }
    let (log, detail, token) =
        load_request_log_for_user(&app_state, &claims, payload.source_request_id).await?;
    ensure_request_can_be_source(&log, detail.as_ref())?;
    let detail = detail.expect("validated source detail must exist");
    let token =
        token.ok_or_else(|| GatewayError::Config("当前请求缺少可用令牌，无法加入实验".into()))?;
    let snapshot = snapshot_from_detail(&detail)?;
    let compare_id = format!("cmp_{}", Uuid::new_v4().simple());
    let created_at = Utc::now();
    let top_p = payload.top_p.clone();
    let presence_penalty = payload.presence_penalty.clone();
    let frequency_penalty = payload.frequency_penalty.clone();
    let preserve_system_prompt = payload.preserve_system_prompt;
    let preserve_message_structure = payload.preserve_message_structure;
    let futures = payload.models.iter().cloned().map(|model| {
        let app_state = Arc::clone(&app_state);
        let token = token.token.clone();
        let snapshot = snapshot.clone();
        let temperature = payload.temperature.clone();
        let max_tokens = payload.max_tokens.clone();
        let top_p = top_p.clone();
        let presence_penalty = presence_penalty.clone();
        let frequency_penalty = frequency_penalty.clone();
        async move {
            let overrides = ReplayOverrideInput {
                model: Some(model.clone()),
                temperature,
                top_p,
                max_tokens,
                presence_penalty,
                frequency_penalty,
                preserve_system_prompt,
                preserve_message_structure,
                ..ReplayOverrideInput::default()
            };
            let result = request_from_snapshot(&snapshot, &overrides)
                .and_then(|(request, top_k)| Ok((request.model.clone(), request, top_k)));
            match result {
                Ok((requested_model, request, top_k)) => {
                    let snapshot_json = match build_request_payload_snapshot(&request, top_k) {
                        Ok(value) => value,
                        Err(err) => {
                            return Ok(failed_compare_item(
                                None,
                                requested_model.clone(),
                                requested_model,
                                None,
                                None,
                                0,
                                None,
                                None,
                                None,
                                None,
                                None,
                                &err,
                            ));
                        }
                    };
                    let executed = execute_logged_chat_request(
                        &app_state,
                        Utc::now(),
                        request,
                        top_k,
                        &token,
                        "/me/compare",
                        REQ_TYPE_CHAT_COMPARE,
                        Some(snapshot_json),
                    )
                    .await;
                    let item = match executed {
                        Ok(executed) => {
                            compare_item_response_from_execution(
                                &app_state,
                                requested_model,
                                executed,
                            )
                            .await?
                        }
                        Err(err) => failed_compare_item(
                            None,
                            requested_model.clone(),
                            requested_model,
                            None,
                            None,
                            0,
                            None,
                            None,
                            None,
                            None,
                            None,
                            &err,
                        ),
                    };
                    Ok::<CompareItemResponse, GatewayError>(item)
                }
                Err(err) => Ok(failed_compare_item(
                    None,
                    model.clone(),
                    model,
                    None,
                    None,
                    0,
                    None,
                    None,
                    None,
                    None,
                    None,
                    &err,
                )),
            }
        }
    });
    let items = join_all(futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, GatewayError>>()?;
    let response = CompareResponse {
        id: compare_id.clone(),
        source_request_id: payload.source_request_id,
        created_at: created_at.to_rfc3339(),
        items,
    };
    app_state
        .log_store
        .save_compare_run(StoredCompareRun {
            id: compare_id,
            user_id: claims.sub,
            source_request_id: payload.source_request_id,
            created_at,
            result_json: serde_json::to_string(&response)?,
        })
        .await
        .map_err(GatewayError::Db)?;
    Ok(Json(compare_detail_response(
        response,
        &log,
        Some(&detail),
    )?))
}

pub async fn get_compare(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(compare_id): Path<String>,
) -> Result<Json<CompareDetailResponse>, GatewayError> {
    let claims = require_user(&headers)?;
    let run = app_state
        .log_store
        .get_compare_run(&compare_id)
        .await
        .map_err(GatewayError::Db)?
        .ok_or_else(|| GatewayError::NotFound("对比记录不存在".into()))?;
    if run.user_id != claims.sub && !is_superadmin(&claims) {
        return Err(GatewayError::Forbidden("无权访问该对比记录".into()));
    }
    let response: CompareResponse = serde_json::from_str(&run.result_json)
        .map_err(|_| GatewayError::Config("对比记录已损坏".into()))?;
    let (log, detail) = if run.user_id == claims.sub {
        let (log, detail, _) =
            load_request_log_for_user(&app_state, &claims, run.source_request_id).await?;
        (log, detail)
    } else {
        let log = app_state
            .log_store
            .get_request_log_by_id(run.source_request_id)
            .await
            .map_err(GatewayError::Db)?
            .ok_or_else(|| GatewayError::NotFound("来源请求不存在".into()))?;
        let detail = app_state
            .log_store
            .get_request_log_detail(run.source_request_id)
            .await
            .map_err(GatewayError::Db)?;
        (log, detail)
    };
    Ok(Json(compare_detail_response(
        response,
        &log,
        detail.as_ref(),
    )?))
}

#[cfg(test)]
mod tests {
    use super::{
        CreateRequestLabTemplateRequest, ListRequestLabTemplatesQuery, ReplayOverrideInput,
        ReplayableRequestSnapshot, UpdateRequestLabSnapshotNoteRequest,
        UpdateRequestLabTemplateRequest, compare_detail_response, compare_item_error_info,
        create_request_lab_template, delete_request_lab_template, detail_response,
        get_my_request_detail, get_request_lab_template, list_request_lab_templates,
        request_from_snapshot, request_lab_snapshot_detail_response, snapshot_matches_keyword,
        update_request_lab_snapshot_note, update_request_lab_template,
    };
    use crate::admin::CreateTokenPayload;
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::error::GatewayError;
    use crate::logging::DatabaseLogger;
    use crate::logging::types::{
        RequestLabExperimentConfig, RequestLog, RequestLogDetailRecord, StoredCompareRun,
        StoredRequestLabSnapshot,
    };
    use crate::server::AppState;
    use crate::server::handlers::auth::{AccessTokenClaims, issue_access_token};
    use crate::server::login::LoginManager;
    use crate::server::storage_traits::RequestLogStore;
    use crate::users::{CreateUserPayload, UserRole, UserStatus};
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use axum::{
        Json,
        extract::{Path, Query, State},
    };
    use chrono::Duration;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Once;
    use tempfile::tempdir;

    fn test_settings(db_path: String) -> crate::config::Settings {
        crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path,
                ..Default::default()
            },
        }
    }

    async fn test_app_state() -> Arc<AppState> {
        let db_path = std::env::temp_dir().join(format!(
            "request_lab_test_{}.db",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let settings = test_settings(db_path.to_str().unwrap().to_string());
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

        Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            organizations: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        })
    }

    fn ensure_test_jwt_secret() {
        static JWT_SECRET_ONCE: Once = Once::new();
        JWT_SECRET_ONCE.call_once(|| unsafe {
            std::env::set_var("GW_JWT_SECRET", "request-lab-test-secret");
        });
    }

    fn auth_headers(user_id: &str, role: &str) -> HeaderMap {
        ensure_test_jwt_secret();
        let claims = AccessTokenClaims {
            sub: user_id.to_string(),
            email: format!("{user_id}@example.com"),
            role: role.to_string(),
            permissions: Vec::new(),
            jti: None,
            exp: (Utc::now() + Duration::hours(1)).timestamp(),
            iat: Some(Utc::now().timestamp()),
        };
        let token = issue_access_token(&claims).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    async fn seed_owned_source_request(app_state: &Arc<AppState>) -> (String, i64, String) {
        let nonce = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let user = app_state
            .user_store
            .create_user(CreateUserPayload {
                first_name: Some("Request".into()),
                last_name: Some("Lab".into()),
                username: Some(format!("request_lab_{nonce}")),
                email: format!("request-lab-{nonce}@example.com"),
                phone_number: None,
                password: Some("password123".into()),
                status: UserStatus::Active,
                role: UserRole::Admin,
                is_anonymous: false,
            })
            .await
            .unwrap();
        let token = app_state
            .token_store
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("Lab Token".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let request_id = app_state
            .log_store
            .log_request(RequestLog {
                id: None,
                timestamp: Utc::now(),
                method: "POST".into(),
                path: "/v1/chat/completions".into(),
                request_type: "chat_once".into(),
                requested_model: Some("openai/gpt-4o-mini".into()),
                effective_model: Some("gpt-4o-mini".into()),
                model: Some("gpt-4o-mini".into()),
                provider: Some("openai".into()),
                api_key: Some("sk-****".into()),
                client_token: Some(token.id.clone()),
                user_id: Some(user.id.clone()),
                amount_spent: Some(0.01),
                status_code: 200,
                response_time_ms: 123,
                prompt_tokens: Some(12),
                completion_tokens: Some(34),
                total_tokens: Some(46),
                cached_tokens: None,
                reasoning_tokens: None,
                error_message: None,
            })
            .await
            .unwrap();

        app_state
            .log_store
            .upsert_request_log_detail(RequestLogDetailRecord {
                request_log_id: request_id,
                request_payload_snapshot: Some(
                    json!({
                        "kind": "chat_completions",
                        "request": {
                            "model": "openai/gpt-4o-mini",
                            "temperature": 0.3,
                            "top_p": 0.9,
                            "max_tokens": 256,
                            "messages": [
                                {"role": "system", "content": "You are helpful."},
                                {"role": "user", "content": "真实输入正文：请总结这段日志。"},
                                {"role": "assistant", "content": "收到，请提供日志。"}
                            ]
                        },
                        "top_k": 2
                    })
                    .to_string(),
                ),
                response_preview: Some("source preview".into()),
                upstream_status: Some(200),
                fallback_triggered: Some(false),
                fallback_reason: None,
                selected_provider: Some("openai".into()),
                selected_key_id: Some("sk-****".into()),
                first_token_latency_ms: Some(66),
            })
            .await
            .unwrap();

        (user.id, request_id, token.id)
    }

    #[test]
    fn request_snapshot_applies_model_and_sampling_overrides() {
        let snapshot = ReplayableRequestSnapshot {
            kind: "chat_completions".into(),
            request: json!({
                "model": "openai/gpt-4o-mini",
                "messages": [{"role": "user", "content": "hello"}],
                "temperature": 0.2,
                "max_tokens": 128,
                "stream": true
            }),
            top_k: Some(4),
        };

        let (request, top_k) = request_from_snapshot(
            &snapshot,
            &ReplayOverrideInput {
                model: Some("openai/gpt-4.1-mini".into()),
                temperature: Some(json!(0.9)),
                top_p: None,
                max_tokens: Some(json!(256)),
                presence_penalty: None,
                frequency_penalty: None,
                preserve_system_prompt: None,
                preserve_message_structure: None,
            },
        )
        .unwrap();

        assert_eq!(request.model, "openai/gpt-4.1-mini");
        assert_eq!(request.temperature, Some(0.9));
        assert_eq!(request.max_tokens, Some(256));
        assert_eq!(request.stream, Some(false));
        assert_eq!(top_k, Some(4));
    }

    #[test]
    fn request_snapshot_rejects_unknown_kind() {
        let snapshot = ReplayableRequestSnapshot {
            kind: "images".into(),
            request: json!({}),
            top_k: None,
        };

        let err = request_from_snapshot(&snapshot, &ReplayOverrideInput::default()).unwrap_err();
        assert!(err.to_string().contains("暂不支持回放"));
    }

    #[test]
    fn snapshot_keyword_matches_note_and_models() {
        let snapshot = StoredRequestLabSnapshot {
            id: "snap_1".into(),
            user_id: "u1".into(),
            source_request_id: 1,
            compare_run_id: "cmp_1".into(),
            note: Some("验证 gpt-4.1 与 claude".into()),
            created_at: Utc::now(),
            snapshot_json: "{}".into(),
            source_requested_model: Some("openai/gpt-4o-mini".into()),
            source_effective_model: Some("openai/gpt-4o-mini".into()),
            models: vec![
                "openai/gpt-4.1-mini".into(),
                "anthropic/claude-3.7-sonnet".into(),
            ],
            success_count: 1,
            failure_count: 1,
        };

        assert!(snapshot_matches_keyword(&snapshot, "claude"));
        assert!(snapshot_matches_keyword(&snapshot, "验证"));
        assert!(!snapshot_matches_keyword(&snapshot, "gemini"));
    }

    #[test]
    fn snapshot_detail_response_exposes_typed_payload_and_keeps_legacy_json() {
        let created_at = Utc::now();
        let snapshot = StoredRequestLabSnapshot {
            id: "snap_detail".into(),
            user_id: "u1".into(),
            source_request_id: 42,
            compare_run_id: "cmp_detail".into(),
            note: Some("完整详情".into()),
            created_at,
            snapshot_json: json!({
                "compare": {
                    "id": "cmp_detail",
                    "source_request_id": 42,
                    "created_at": created_at.to_rfc3339(),
                    "items": [
                        {
                            "request_id": 1001,
                            "model": "openai/gpt-4.1-mini",
                            "requested_model": "openai/gpt-4.1-mini",
                            "effective_model": "gpt-4.1-mini",
                            "provider": "openai",
                            "output_summary": "# title",
                            "response": {"choices": []},
                            "response_time_ms": 321,
                            "input_tokens": 12,
                            "output_tokens": 34,
                            "total_tokens": 46,
                            "cost": 0.00123,
                            "status": "success",
                            "status_code": 200
                        }
                    ]
                },
                "source": {
                    "source_request_id": 42,
                    "requested_model": "openai/gpt-4o-mini",
                    "effective_model": "gpt-4o-mini",
                    "provider": "openai",
                    "method": "POST",
                    "path": "/v1/chat/completions",
                    "status": "success",
                    "status_code": 200,
                    "source_timestamp": "2025-01-01T00:00:00Z",
                    "request_payload_snapshot": {
                        "kind": "chat_completions",
                        "request": {
                            "model": "openai/gpt-4o-mini",
                            "messages": [{"role": "user", "content": "hello"}]
                        },
                        "top_k": 4
                    },
                    "response_preview": "hello world"
                }
            })
            .to_string(),
            source_requested_model: Some("openai/gpt-4o-mini".into()),
            source_effective_model: Some("gpt-4o-mini".into()),
            models: vec!["openai/gpt-4.1-mini".into()],
            success_count: 1,
            failure_count: 0,
        };

        let response = request_lab_snapshot_detail_response(snapshot).unwrap();

        assert_eq!(response.compare.id, "cmp_detail");
        assert_eq!(response.compare.items.len(), 1);
        assert_eq!(response.compare.items[0].model, "openai/gpt-4.1-mini");
        assert_eq!(response.source.source_request_id, 42);
        assert_eq!(
            response
                .source
                .request_payload_snapshot
                .as_ref()
                .and_then(|item| item.top_k),
            Some(4)
        );
        assert_eq!(
            response.snapshot_json["source"]["response_preview"],
            json!("hello world")
        );
    }

    #[test]
    fn detail_response_keeps_preview_when_snapshot_is_invalid() {
        let log = RequestLog {
            id: Some(42),
            timestamp: Utc::now(),
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            request_type: "chat_once".into(),
            requested_model: Some("openai/gpt-4o-mini".into()),
            effective_model: Some("gpt-4o-mini".into()),
            model: Some("gpt-4o-mini".into()),
            provider: Some("openai".into()),
            api_key: Some("sk-****".into()),
            client_token: Some("tok_1".into()),
            user_id: None,
            amount_spent: Some(0.01),
            status_code: 200,
            response_time_ms: 123,
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            total_tokens: Some(15),
            cached_tokens: None,
            reasoning_tokens: None,
            error_message: None,
        };
        let detail = RequestLogDetailRecord {
            request_log_id: 42,
            request_payload_snapshot: Some("{not-json".into()),
            response_preview: Some("preview text".into()),
            upstream_status: Some(200),
            fallback_triggered: Some(false),
            fallback_reason: None,
            selected_provider: Some("openai".into()),
            selected_key_id: Some("sk-****".into()),
            first_token_latency_ms: Some(88),
        };

        let response = detail_response(
            log,
            Some(detail),
            Some("Token A".into()),
            Some("alice".into()),
        )
        .unwrap();

        assert_eq!(response.response_preview.as_deref(), Some("preview text"));
        assert!(response.request_payload_snapshot.is_none());
        assert_eq!(response.selected_provider.as_deref(), Some("openai"));
    }

    #[test]
    fn compare_detail_response_exposes_source_preview_and_snapshot_json() {
        let now = Utc::now();
        let log = RequestLog {
            id: Some(77),
            timestamp: now,
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            request_type: "chat_once".into(),
            requested_model: Some("openai/gpt-4o-mini".into()),
            effective_model: Some("gpt-4o-mini".into()),
            model: Some("gpt-4o-mini".into()),
            provider: Some("openai".into()),
            api_key: None,
            client_token: Some("tok_1".into()),
            user_id: Some("u1".into()),
            amount_spent: Some(0.02),
            status_code: 200,
            response_time_ms: 120,
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            total_tokens: Some(30),
            cached_tokens: None,
            reasoning_tokens: None,
            error_message: None,
        };
        let detail = RequestLogDetailRecord {
            request_log_id: 77,
            request_payload_snapshot: Some(
                json!({
                    "kind": "chat_completions",
                    "request": {
                        "model": "openai/gpt-4o-mini",
                        "messages": [{"role": "user", "content": "hello"}]
                    },
                    "top_k": 2
                })
                .to_string(),
            ),
            response_preview: Some("source preview".into()),
            upstream_status: Some(200),
            fallback_triggered: Some(false),
            fallback_reason: None,
            selected_provider: Some("openai".into()),
            selected_key_id: Some("sk-****".into()),
            first_token_latency_ms: Some(45),
        };
        let compare = super::CompareResponse {
            id: "cmp_live".into(),
            source_request_id: 77,
            created_at: now.to_rfc3339(),
            items: vec![super::CompareItemResponse {
                request_id: Some(99),
                model: "openai/gpt-5.4".into(),
                requested_model: "openai/gpt-5.4".into(),
                effective_model: Some("gpt-5.4".into()),
                provider: Some("openai".into()),
                output_summary: Some("hello".into()),
                response: Some(json!({"choices": []})),
                response_time_ms: 321,
                input_tokens: Some(11),
                output_tokens: Some(22),
                total_tokens: Some(33),
                cost: Some(0.1),
                status: "success".into(),
                status_code: 200,
                error_message: None,
                error: None,
                upstream_status: Some(200),
                selected_provider: Some("openai".into()),
                selected_key_id: Some("sk-****".into()),
                first_token_latency_ms: Some(21),
            }],
        };

        let response = compare_detail_response(compare, &log, Some(&detail)).unwrap();

        assert_eq!(response.id, "cmp_live");
        assert_eq!(response.compare_run_id, "cmp_live");
        assert_eq!(response.models, vec!["openai/gpt-5.4"]);
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items_summary.total_count, 1);
        assert_eq!(
            response.source.response_preview.as_deref(),
            Some("source preview")
        );
        assert_eq!(
            response.snapshot_json["source"]["response_preview"],
            json!("source preview")
        );
        assert_eq!(
            response.snapshot_json["compare"]["items"][0]["model"],
            json!("openai/gpt-5.4")
        );
    }

    #[test]
    fn compare_item_error_info_localizes_balance_errors() {
        let error = compare_item_error_info(&GatewayError::Config(
            "Config error: Insufficient account balance".into(),
        ));

        assert_eq!(error.code, "insufficient_balance");
        assert_eq!(
            error.i18n_key,
            "request_lab.compare.error.insufficient_balance"
        );
        assert_eq!(
            error.message,
            "余额不足，请充值后重试。若当前令牌已被停用，请在充值或订阅后手动启用。"
        );
        assert_eq!(
            error.localized_message.en,
            "Insufficient balance. Please top up and try again. If the token was disabled, re-enable it after topping up or renewing your subscription."
        );
        assert_eq!(
            error.detail.as_deref(),
            Some("Insufficient account balance")
        );
    }

    #[tokio::test]
    async fn snapshot_storage_roundtrip_preserves_compare_run() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let created_at = Utc::now();

        RequestLogStore::save_compare_run(
            &db,
            StoredCompareRun {
                id: "cmp_1".into(),
                user_id: "u1".into(),
                source_request_id: 42,
                created_at,
                result_json: json!({
                    "id": "cmp_1",
                    "source_request_id": 42,
                    "created_at": created_at.to_rfc3339(),
                    "items": []
                })
                .to_string(),
            },
        )
        .await
        .unwrap();

        let snapshot = StoredRequestLabSnapshot {
            id: "snap_1".into(),
            user_id: "u1".into(),
            source_request_id: 42,
            compare_run_id: "cmp_1".into(),
            note: Some("第一次保存".into()),
            created_at,
            snapshot_json: json!({
                "compare": {
                    "id": "cmp_1",
                    "source_request_id": 42,
                    "created_at": created_at.to_rfc3339(),
                    "items": []
                },
                "source": {
                    "source_request_id": 42
                }
            })
            .to_string(),
            source_requested_model: Some("openai/gpt-4o-mini".into()),
            source_effective_model: Some("openai/gpt-4o-mini".into()),
            models: vec![
                "openai/gpt-4.1-mini".into(),
                "anthropic/claude-3.7-sonnet".into(),
            ],
            success_count: 1,
            failure_count: 1,
        };

        RequestLogStore::save_request_lab_snapshot(&db, snapshot.clone())
            .await
            .unwrap();

        let listed = RequestLogStore::list_request_lab_snapshots(&db, "u1")
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, snapshot.id);

        let stored = RequestLogStore::get_request_lab_snapshot(&db, "snap_1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.compare_run_id, "cmp_1");
        assert_eq!(stored.models.len(), 2);

        let deleted = RequestLogStore::delete_request_lab_snapshot(&db, "u1", "snap_1")
            .await
            .unwrap();
        assert!(deleted);
        assert!(
            RequestLogStore::list_request_lab_snapshots(&db, "u1")
                .await
                .unwrap()
                .is_empty()
        );

        let compare_run = RequestLogStore::get_compare_run(&db, "cmp_1")
            .await
            .unwrap();
        assert!(compare_run.is_some());
    }

    #[tokio::test]
    async fn snapshot_storage_is_idempotent_per_compare_run() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let created_at = Utc::now();

        let first = StoredRequestLabSnapshot {
            id: "snap_a".into(),
            user_id: "u1".into(),
            source_request_id: 7,
            compare_run_id: "cmp_same".into(),
            note: Some("第一次备注".into()),
            created_at,
            snapshot_json: "{}".into(),
            source_requested_model: Some("openai/gpt-4o-mini".into()),
            source_effective_model: Some("openai/gpt-4o-mini".into()),
            models: vec!["openai/gpt-4.1-mini".into()],
            success_count: 1,
            failure_count: 0,
        };
        let second = StoredRequestLabSnapshot {
            id: "snap_b".into(),
            user_id: "u1".into(),
            source_request_id: 7,
            compare_run_id: "cmp_same".into(),
            note: Some("更新后的备注".into()),
            created_at,
            snapshot_json: "{}".into(),
            source_requested_model: Some("openai/gpt-4o-mini".into()),
            source_effective_model: Some("openai/gpt-4o-mini".into()),
            models: vec![
                "openai/gpt-4.1-mini".into(),
                "anthropic/claude-3.7-sonnet".into(),
            ],
            success_count: 2,
            failure_count: 0,
        };

        RequestLogStore::save_request_lab_snapshot(&db, first)
            .await
            .unwrap();
        RequestLogStore::save_request_lab_snapshot(&db, second)
            .await
            .unwrap();

        let listed = RequestLogStore::list_request_lab_snapshots(&db, "u1")
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "snap_a");
        assert_eq!(listed[0].note.as_deref(), Some("更新后的备注"));
        assert_eq!(listed[0].models.len(), 2);

        let by_compare =
            RequestLogStore::get_request_lab_snapshot_by_compare_run(&db, "u1", "cmp_same")
                .await
                .unwrap();
        assert!(by_compare.is_some());
    }

    #[tokio::test]
    async fn snapshot_note_update_roundtrip_trims_and_clears() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let created_at = Utc::now();

        RequestLogStore::save_request_lab_snapshot(
            &db,
            StoredRequestLabSnapshot {
                id: "snap_note".into(),
                user_id: "u1".into(),
                source_request_id: 9,
                compare_run_id: "cmp_note".into(),
                note: Some("初始备注".into()),
                created_at,
                snapshot_json: "{}".into(),
                source_requested_model: Some("openai/gpt-5.2".into()),
                source_effective_model: Some("gpt-5.2".into()),
                models: vec!["openai/gpt-5.4".into()],
                success_count: 1,
                failure_count: 0,
            },
        )
        .await
        .unwrap();

        let trimmed_note = super::normalize_snapshot_note(Some("  新备注  ".into()));
        RequestLogStore::update_request_lab_snapshot_note(&db, "snap_note", trimmed_note)
            .await
            .unwrap();
        let stored = RequestLogStore::get_request_lab_snapshot(&db, "snap_note")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.note.as_deref(), Some("新备注"));

        let listed = RequestLogStore::list_request_lab_snapshots(&db, "u1")
            .await
            .unwrap();
        assert_eq!(listed[0].note.as_deref(), Some("新备注"));

        let by_compare =
            RequestLogStore::get_request_lab_snapshot_by_compare_run(&db, "u1", "cmp_note")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(by_compare.note.as_deref(), Some("新备注"));

        let cleared_note = super::normalize_snapshot_note(Some("   ".into()));
        RequestLogStore::update_request_lab_snapshot_note(&db, "snap_note", cleared_note)
            .await
            .unwrap();
        let cleared = RequestLogStore::get_request_lab_snapshot(&db, "snap_note")
            .await
            .unwrap()
            .unwrap();
        assert!(cleared.note.is_none());
    }

    #[tokio::test]
    async fn snapshot_note_update_rejects_unauthorized_user() {
        let app_state = test_app_state().await;
        RequestLogStore::save_request_lab_snapshot(
            app_state.log_store.as_ref(),
            StoredRequestLabSnapshot {
                id: "snap_forbidden".into(),
                user_id: "owner".into(),
                source_request_id: 11,
                compare_run_id: "cmp_forbidden".into(),
                note: Some("原备注".into()),
                created_at: Utc::now(),
                snapshot_json: "{}".into(),
                source_requested_model: Some("openai/gpt-5.2".into()),
                source_effective_model: Some("gpt-5.2".into()),
                models: vec!["openai/gpt-5.4".into()],
                success_count: 1,
                failure_count: 0,
            },
        )
        .await
        .unwrap();

        let result = update_request_lab_snapshot_note(
            State(app_state),
            auth_headers("other-user", "user"),
            Path("snap_forbidden".to_string()),
            Json(UpdateRequestLabSnapshotNoteRequest {
                note: Some("试图修改".into()),
            }),
        )
        .await;

        assert!(matches!(result, Err(GatewayError::Forbidden(_))));
    }

    #[tokio::test]
    async fn request_detail_response_returns_structured_system_and_messages() {
        let app_state = test_app_state().await;
        let (user_id, request_id, _) = seed_owned_source_request(&app_state).await;

        let Json(response) = get_my_request_detail(
            State(app_state),
            auth_headers(&user_id, "user"),
            Path(request_id),
        )
        .await
        .unwrap();

        assert_eq!(
            response
                .system_prompt
                .as_ref()
                .and_then(|item| item.text.as_deref()),
            Some("You are helpful.")
        );
        assert_eq!(response.messages.len(), 2);
        assert_eq!(response.messages[0].role, "user");
        assert_eq!(
            response.messages[0].content.text.as_deref(),
            Some("真实输入正文：请总结这段日志。")
        );
        assert_eq!(
            response.source_request_summary.source_request_id,
            request_id
        );
        assert!(response.locked_fields.iter().any(|item| item == "messages"));
    }

    #[tokio::test]
    async fn request_lab_template_crud_roundtrip_works() {
        let app_state = test_app_state().await;
        let (user_id, request_id, _) = seed_owned_source_request(&app_state).await;

        let (status, Json(created)) = create_request_lab_template(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Json(CreateRequestLabTemplateRequest {
                scope: None,
                name: "产品分析模板".into(),
                description: Some("保留真实输入，只切换实验方法".into()),
                tags: vec!["analysis".into(), "review".into()],
                source_request_id: request_id,
                compare_models: vec![
                    "openai/gpt-5.4".into(),
                    "anthropic/claude-3.7-sonnet".into(),
                ],
                experiment_config: RequestLabExperimentConfig {
                    temperature: Some(json!(0.4)),
                    top_p: Some(json!(0.8)),
                    max_tokens: Some(json!(512)),
                    presence_penalty: Some(json!(0.1)),
                    frequency_penalty: Some(json!(0.2)),
                    preserve_system_prompt: true,
                    preserve_message_structure: true,
                },
            }),
        )
        .await
        .unwrap();

        assert_eq!(status, axum::http::StatusCode::CREATED);
        assert_eq!(created.scope, "personal");
        assert_eq!(created.compare_models.len(), 2);
        assert_eq!(created.experiment_config.temperature, Some(json!(0.4)));

        let Json(listed) = list_request_lab_templates(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Query(ListRequestLabTemplatesQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let Json(updated_meta) = update_request_lab_template(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Path(created.id.clone()),
            Json(UpdateRequestLabTemplateRequest {
                name: Some("产品分析模板 v2".into()),
                description: Some("只更新模板说明".into()),
                tags: Some(vec!["analysis".into(), "v2".into()]),
                source_request_id: None,
                compare_models: None,
                experiment_config: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated_meta.name, "产品分析模板 v2");
        assert_eq!(updated_meta.compare_models, created.compare_models);

        let Json(updated_method) = update_request_lab_template(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Path(created.id.clone()),
            Json(UpdateRequestLabTemplateRequest {
                name: None,
                description: None,
                tags: None,
                source_request_id: Some(request_id),
                compare_models: Some(vec![
                    "openai/gpt-5.4".into(),
                    "google/gemini-2.5-pro".into(),
                ]),
                experiment_config: Some(RequestLabExperimentConfig {
                    temperature: Some(json!(0.2)),
                    top_p: Some(json!(0.7)),
                    max_tokens: Some(json!(768)),
                    presence_penalty: Some(json!(0.0)),
                    frequency_penalty: Some(json!(0.0)),
                    preserve_system_prompt: false,
                    preserve_message_structure: false,
                }),
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated_method.name, "产品分析模板 v2");
        assert_eq!(
            updated_method.compare_models,
            vec!["openai/gpt-5.4", "google/gemini-2.5-pro"]
        );
        assert!(!updated_method.experiment_config.preserve_system_prompt);
        assert!(!updated_method.experiment_config.preserve_message_structure);

        let Json(fetched) = get_request_lab_template(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Path(created.id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(fetched.id, created.id);

        let Json(deleted) = delete_request_lab_template(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Path(created.id),
        )
        .await
        .unwrap();
        assert!(deleted.deleted);
    }

    #[tokio::test]
    async fn request_lab_template_list_supports_keyword_and_tag_filters() {
        let app_state = test_app_state().await;
        let (user_id, request_id, _) = seed_owned_source_request(&app_state).await;

        for (name, tags, models) in [
            (
                "产品评审模板",
                vec!["review".to_string(), "analysis".to_string()],
                vec![
                    "openai/gpt-5.4".to_string(),
                    "anthropic/claude-3.7-sonnet".to_string(),
                ],
            ),
            (
                "低成本筛选模板",
                vec!["cheap".to_string()],
                vec![
                    "openai/gpt-4.1-mini".to_string(),
                    "openai/gpt-4o-mini".to_string(),
                ],
            ),
        ] {
            let _ = create_request_lab_template(
                State(app_state.clone()),
                auth_headers(&user_id, "user"),
                Json(CreateRequestLabTemplateRequest {
                    scope: None,
                    name: name.into(),
                    description: None,
                    tags,
                    source_request_id: request_id,
                    compare_models: models,
                    experiment_config: RequestLabExperimentConfig::default(),
                }),
            )
            .await
            .unwrap();
        }

        let Json(keyword_filtered) = list_request_lab_templates(
            State(app_state.clone()),
            auth_headers(&user_id, "user"),
            Query(ListRequestLabTemplatesQuery {
                keyword: Some("评审".into()),
                tag: None,
                sort: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(keyword_filtered.len(), 1);
        assert_eq!(keyword_filtered[0].name, "产品评审模板");

        let Json(tag_filtered) = list_request_lab_templates(
            State(app_state),
            auth_headers(&user_id, "user"),
            Query(ListRequestLabTemplatesQuery {
                keyword: None,
                tag: Some("cheap".into()),
                sort: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(tag_filtered.len(), 1);
        assert_eq!(tag_filtered[0].name, "低成本筛选模板");
    }

    #[tokio::test]
    async fn request_lab_template_rejects_cross_user_access_and_never_stores_real_prompt_body() {
        let app_state = test_app_state().await;
        let (owner_id, request_id, _) = seed_owned_source_request(&app_state).await;
        let (status, Json(created)) = create_request_lab_template(
            State(app_state.clone()),
            auth_headers(&owner_id, "user"),
            Json(CreateRequestLabTemplateRequest {
                scope: None,
                name: "长文本审阅模板".into(),
                description: Some("不会保存真实输入正文".into()),
                tags: vec!["review".into()],
                source_request_id: request_id,
                compare_models: vec![
                    "openai/gpt-5.4".into(),
                    "anthropic/claude-3.7-sonnet".into(),
                ],
                experiment_config: RequestLabExperimentConfig::default(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, axum::http::StatusCode::CREATED);

        let (other_user_id, _, _) = seed_owned_source_request(&app_state).await;
        let result = get_request_lab_template(
            State(app_state.clone()),
            auth_headers(&other_user_id, "user"),
            Path(created.id.clone()),
        )
        .await;
        assert!(matches!(result, Err(GatewayError::Forbidden(_))));

        let template_json = serde_json::to_string(&created).unwrap();
        assert!(!template_json.contains("真实输入正文：请总结这段日志。"));
        assert!(!template_json.contains("You are helpful."));
    }
}
