use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::server::util::{bearer_token, token_for_log};
use crate::{
    admin::{AdminToken, CreateTokenPayload, UpdateTokenPayload},
    error::GatewayError,
    server::AppState,
};

#[derive(Debug, Serialize)]
pub struct AdminTokenOut {
    pub token: String,
    pub allowed_models: Option<Vec<String>>,
    pub max_tokens: Option<i64>,
    pub max_amount: Option<f64>,
    pub amount_spent: f64,
    pub prompt_tokens_spent: i64,
    pub completion_tokens_spent: i64,
    pub total_tokens_spent: i64,
    pub usage_count: i64,
    pub enabled: bool,
    pub expires_at: Option<String>, // 以北京时间字符串返回
    pub created_at: String,
}

impl From<AdminToken> for AdminTokenOut {
    fn from(t: AdminToken) -> Self {
        Self {
            token: t.token,
            allowed_models: t.allowed_models,
            max_tokens: t.max_tokens,
            max_amount: t.max_amount,
            amount_spent: t.amount_spent,
            prompt_tokens_spent: t.prompt_tokens_spent,
            completion_tokens_spent: t.completion_tokens_spent,
            total_tokens_spent: t.total_tokens_spent,
            usage_count: 0,
            enabled: t.enabled,
            expires_at: t
                .expires_at
                .as_ref()
                .map(|dt| crate::logging::time::to_beijing_string(dt)),
            created_at: crate::logging::time::to_beijing_string(&t.created_at),
        }
    }
}

use super::auth::ensure_admin;
use crate::server::request_logging::log_simple_request;
use chrono::Utc;

pub async fn list_tokens(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminTokenOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/tokens",
            "admin_tokens_list",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    use std::collections::HashMap;
    let usage_counts: HashMap<String, i64> = app_state
        .log_store
        .count_requests_by_client_token()
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .collect();
    let tokens = app_state
        .token_store
        .list_tokens()
        .await?
        .into_iter()
        .map(|token| {
            let mut out = AdminTokenOut::from(token.clone());
            if let Some(count) = usage_counts.get(&token.token) {
                out.usage_count = *count;
            }
            out
        })
        .collect();
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/tokens",
        "admin_tokens_list",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(tokens))
}

pub async fn get_token(
    Path(token): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/admin/tokens/{}", token),
            "admin_tokens_get",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    match app_state.token_store.get_token(&token).await? {
        Some(t) => {
            let mut out = AdminTokenOut::from(t.clone());
            if let Some(count) = app_state
                .log_store
                .count_requests_by_client_token()
                .await
                .map_err(GatewayError::Db)?
                .into_iter()
                .find(|(tok, _)| tok == &token)
                .map(|(_, c)| c)
            {
                out.usage_count = count;
            }
            Ok(Json(out))
        }
        None => {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &format!("/admin/tokens/{}", token),
                "admin_tokens_get",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            Err(ge)
        }
    }
}

pub async fn create_token(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateTokenPayload>,
) -> Result<(axum::http::StatusCode, Json<AdminTokenOut>), GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/tokens",
            "admin_tokens_create",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    // 校验 allowed_models 存在性（若提供）
    if let Some(list) = payload.allowed_models.as_ref() {
        if !list.is_empty() {
            use std::collections::HashSet;
            let cached = crate::server::model_cache::get_cached_models_all(&app_state)
                .await
                .map_err(GatewayError::Db)?;
            let set: HashSet<String> = cached.into_iter().map(|m| m.id).collect();
            for m in list {
                if !set.contains(m) {
                    return Err(GatewayError::NotFound(format!(
                        "model '{}' not found in cache",
                        m
                    )));
                }
            }
        }
    }
    let t = app_state
        .token_store
        .create_token(CreateTokenPayload {
            token: None,
            ..payload
        })
        .await?;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/tokens",
        "admin_tokens_create",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        201,
        None,
    )
    .await;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(AdminTokenOut::from(t)),
    ))
}

#[derive(Debug, Deserialize)]
pub struct TogglePayload {
    pub enabled: bool,
}

pub async fn toggle_token(
    Path(token): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<TogglePayload>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/admin/tokens/{}/toggle", token),
            "admin_tokens_toggle",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    let ok = app_state
        .token_store
        .set_enabled(&token, payload.enabled)
        .await?;
    if ok {
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/admin/tokens/{}/toggle", token),
            "admin_tokens_toggle",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        Ok(Json(serde_json::json!({"status":"ok"})))
    } else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            &format!("/admin/tokens/{}/toggle", token),
            "admin_tokens_toggle",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        Err(ge)
    }
}

pub async fn delete_token(
    Path(token): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/admin/tokens/{}", token),
            "admin_tokens_delete",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    let deleted = app_state.token_store.delete_token(&token).await?;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/admin/tokens/{}", token),
            "admin_tokens_delete",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            204,
            None,
        )
        .await;
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/admin/tokens/{}", token),
            "admin_tokens_delete",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        Err(ge)
    }
}

pub async fn update_token(
    Path(token): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateTokenPayload>,
) -> Result<Json<AdminTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/admin/tokens/{}", token),
            "admin_tokens_update",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    // 若更新了 allowed_models，需要校验
    if let Some(list) = payload.allowed_models.as_ref() {
        use std::collections::HashSet;
        let cached = crate::server::model_cache::get_cached_models_all(&app_state)
            .await
            .map_err(GatewayError::Db)?;
        let set: HashSet<String> = cached.into_iter().map(|m| m.id).collect();
        for m in list {
            if !set.contains(m) {
                return Err(GatewayError::NotFound(format!(
                    "model '{}' not found in cache",
                    m
                )));
            }
        }
    }
    match app_state.token_store.update_token(&token, payload).await? {
        Some(t) => {
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                &format!("/admin/tokens/{}", token),
                "admin_tokens_update",
                None,
                None,
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            Ok(Json(AdminTokenOut::from(t)))
        }
        None => {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                &format!("/admin/tokens/{}", token),
                "admin_tokens_update",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            Err(ge)
        }
    }
}
