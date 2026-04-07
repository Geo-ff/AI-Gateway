use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
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
    REQ_TYPE_CHAT_COMPARE, REQ_TYPE_CHAT_REPLAY, RequestLogDetailRecord, StoredCompareRun,
    StoredRequestLabSource,
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
    pub max_tokens: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompareRequest {
    pub source_request_id: i64,
    pub models: Vec<String>,
    #[serde(default)]
    pub temperature: Option<serde_json::Value>,
    #[serde(default)]
    pub max_tokens: Option<serde_json::Value>,
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
    pub fallback_triggered: bool,
    pub error_message: Option<String>,
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
    pub fallback_triggered: Option<bool>,
    pub fallback_reason: Option<String>,
    pub selected_provider: Option<String>,
    pub selected_key_id: Option<String>,
    pub first_token_latency_ms: Option<i64>,
    pub error_message: Option<String>,
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
    pub fallback_triggered: bool,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub struct ExecutedChatRequest {
    pub requested_model: String,
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
    if let Some(max_tokens) = overrides.max_tokens.clone() {
        request_obj.insert("max_tokens".to_string(), max_tokens);
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
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| GatewayError::Config("请求快照格式非法".into()))?;
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
        fallback_triggered: detail.as_ref().and_then(|item| item.fallback_triggered),
        fallback_reason: detail
            .as_ref()
            .and_then(|item| item.fallback_reason.clone()),
        selected_provider: detail
            .as_ref()
            .and_then(|item| item.selected_provider.clone()),
        selected_key_id: detail
            .as_ref()
            .and_then(|item| item.selected_key_id.clone()),
        first_token_latency_ms: detail.as_ref().and_then(|item| item.first_token_latency_ms),
        error_message: log.error_message,
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
            fallback_triggered: Some(false),
            fallback_reason: None,
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
        requested_model,
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
                fallback_triggered: false,
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
            fallback_triggered: false,
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

pub async fn create_compare(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CompareRequest>,
) -> Result<Json<CompareResponse>, GatewayError> {
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
    let futures = payload.models.iter().cloned().map(|model| {
        let app_state = Arc::clone(&app_state);
        let token = token.token.clone();
        let snapshot = snapshot.clone();
        let temperature = payload.temperature.clone();
        let max_tokens = payload.max_tokens.clone();
        async move {
            let overrides = ReplayOverrideInput {
                model: Some(model.clone()),
                temperature,
                max_tokens,
                ..ReplayOverrideInput::default()
            };
            let result = request_from_snapshot(&snapshot, &overrides)
                .and_then(|(request, top_k)| Ok((request.model.clone(), request, top_k)));
            match result {
                Ok((requested_model, request, top_k)) => {
                    let snapshot_json = match build_request_payload_snapshot(&request, top_k) {
                        Ok(value) => value,
                        Err(err) => {
                            return Ok(CompareItemResponse {
                                request_id: None,
                                model: requested_model.clone(),
                                requested_model,
                                effective_model: None,
                                provider: None,
                                output_summary: None,
                                response: None,
                                response_time_ms: 0,
                                input_tokens: None,
                                output_tokens: None,
                                total_tokens: None,
                                cost: None,
                                status: "failed".to_string(),
                                status_code: err.status_code().as_u16(),
                                fallback_triggered: false,
                                error_message: Some(err.to_string()),
                            });
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
                        Ok(executed) => match &executed.response {
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
                                    output_tokens: usage
                                        .as_ref()
                                        .map(|usage| usage.completion_tokens),
                                    total_tokens: usage.as_ref().map(|usage| usage.total_tokens),
                                    cost: executed.logged.amount_spent,
                                    status: "success".to_string(),
                                    status_code: 200,
                                    fallback_triggered: false,
                                    error_message: None,
                                }
                            }
                            Err(err) => CompareItemResponse {
                                request_id: executed.logged.log_id,
                                model: requested_model.clone(),
                                requested_model,
                                effective_model: Some(executed.effective_model.clone()),
                                provider: Some(executed.provider_name.clone()),
                                output_summary: None,
                                response: None,
                                response_time_ms: executed.logged.response_time_ms,
                                input_tokens: None,
                                output_tokens: None,
                                total_tokens: None,
                                cost: executed.logged.amount_spent,
                                status: "failed".to_string(),
                                status_code: err.status_code().as_u16(),
                                fallback_triggered: false,
                                error_message: Some(err.to_string()),
                            },
                        },
                        Err(err) => CompareItemResponse {
                            request_id: None,
                            model: requested_model.clone(),
                            requested_model,
                            effective_model: None,
                            provider: None,
                            output_summary: None,
                            response: None,
                            response_time_ms: 0,
                            input_tokens: None,
                            output_tokens: None,
                            total_tokens: None,
                            cost: None,
                            status: "failed".to_string(),
                            status_code: err.status_code().as_u16(),
                            fallback_triggered: false,
                            error_message: Some(err.to_string()),
                        },
                    };
                    Ok::<CompareItemResponse, GatewayError>(item)
                }
                Err(err) => Ok(CompareItemResponse {
                    request_id: None,
                    model: model.clone(),
                    requested_model: model,
                    effective_model: None,
                    provider: None,
                    output_summary: None,
                    response: None,
                    response_time_ms: 0,
                    input_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    cost: None,
                    status: "failed".to_string(),
                    status_code: err.status_code().as_u16(),
                    fallback_triggered: false,
                    error_message: Some(err.to_string()),
                }),
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
    Ok(Json(response))
}

pub async fn get_compare(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(compare_id): Path<String>,
) -> Result<Json<CompareResponse>, GatewayError> {
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
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::{ReplayOverrideInput, ReplayableRequestSnapshot, request_from_snapshot};
    use serde_json::json;

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
                max_tokens: Some(json!(256)),
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
}
