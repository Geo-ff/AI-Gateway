use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::auth::require_user;
use crate::admin::{ClientToken, CreateTokenPayload, UpdateTokenPayload};
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Serialize)]
pub struct MyTokenOut {
    pub id: String,
    pub name: String,
    pub allowed_models: Option<Vec<String>>,
    pub model_blacklist: Option<Vec<String>>,
    pub max_tokens: Option<i64>,
    pub max_amount: Option<f64>,
    pub amount_spent: f64,
    pub prompt_tokens_spent: i64,
    pub completion_tokens_spent: i64,
    pub total_tokens_spent: i64,
    pub usage_count: i64,
    pub enabled: bool,
    pub expires_at: Option<String>,
    pub created_at: String,
}

impl From<ClientToken> for MyTokenOut {
    fn from(t: ClientToken) -> Self {
        Self {
            id: t.id,
            name: t.name,
            allowed_models: t.allowed_models,
            model_blacklist: t.model_blacklist,
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
                .map(crate::logging::time::to_iso8601_utc_string),
            created_at: crate::logging::time::to_iso8601_utc_string(&t.created_at),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateMyTokenPayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>,
    #[serde(default)]
    pub model_blacklist: Option<Vec<String>>,
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
    #[serde(default = "default_enabled_true")]
    pub enabled: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
}

fn default_enabled_true() -> bool {
    true
}

fn validate_optional_name(name: Option<String>) -> Result<Option<String>, GatewayError> {
    let Some(name) = name else { return Ok(None) };
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(GatewayError::Config("name 不能为空".into()));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(GatewayError::Config("name 不能包含控制字符".into()));
    }
    if trimmed.chars().count() > 64 {
        return Err(GatewayError::Config("name 长度不能超过 64".into()));
    }
    Ok(Some(trimmed.to_string()))
}

#[derive(Debug, Serialize)]
pub struct CreateMyTokenResponse {
    pub token: String,
    pub token_info: MyTokenOut,
}

pub async fn list_my_tokens(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<MyTokenOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/tokens",
                "me_tokens_list",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

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
        .list_tokens_by_user(&claims.sub)
        .await?
        .into_iter()
        .map(|token| {
            let mut out = MyTokenOut::from(token.clone());
            if let Some(count) = usage_counts.get(&token.id) {
                out.usage_count = *count;
            }
            out
        })
        .collect();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/tokens",
        "me_tokens_list",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(tokens))
}

pub async fn create_my_token(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateMyTokenPayload>,
) -> Result<(axum::http::StatusCode, Json<CreateMyTokenResponse>), GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/me/tokens",
                "me_tokens_create",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let name = validate_optional_name(payload.name)?;
    if let Some(v) = payload.max_amount
        && v < 0.0
    {
        return Err(GatewayError::Config("max_amount 必须 >= 0".into()));
    }
    let allowed_models = crate::server::token_model_limits::normalize_model_list(
        "allowed_models",
        payload.allowed_models,
    )?;
    let model_blacklist = crate::server::token_model_limits::normalize_model_list(
        "model_blacklist",
        payload.model_blacklist,
    )?;
    crate::server::token_model_limits::ensure_model_lists_mutually_exclusive(
        &allowed_models,
        &model_blacklist,
    )?;
    crate::server::token_model_limits::validate_models_exist_in_cache(
        &app_state,
        "allowed_models",
        &allowed_models,
    )
    .await?;
    crate::server::token_model_limits::validate_models_exist_in_cache(
        &app_state,
        "model_blacklist",
        &model_blacklist,
    )
    .await?;

    let created = app_state
        .token_store
        .create_token(CreateTokenPayload {
            id: None,
            user_id: Some(claims.sub.clone()),
            name,
            token: None,
            allowed_models,
            model_blacklist,
            max_tokens: payload.max_tokens,
            max_amount: payload.max_amount,
            enabled: payload.enabled,
            expires_at: payload.expires_at,
            remark: None,
            organization_id: None,
            ip_whitelist: None,
            ip_blacklist: None,
        })
        .await?;

    let token_plain = created.token.clone();
    let token_info = MyTokenOut::from(created);
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/me/tokens",
        "me_tokens_create",
        None,
        None,
        token_for_log(provided.as_deref()),
        201,
        None,
    )
    .await;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(CreateMyTokenResponse {
            token: token_plain,
            token_info,
        }),
    ))
}

pub async fn get_my_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<MyTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/me/tokens/{id}",
                "me_tokens_get",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let Some(t) = app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, &id)
        .await?
    else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/me/tokens/{id}",
            "me_tokens_get",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    };

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/me/tokens/{id}",
        "me_tokens_get",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(MyTokenOut::from(t)))
}

pub async fn update_my_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateTokenPayload>,
) -> Result<Json<MyTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                "/me/tokens/{id}",
                "me_tokens_update",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    if payload.id.is_some() {
        return Err(GatewayError::Config("不允许修改 id".into()));
    }
    let mut payload = payload;
    payload.allowed_models = crate::server::token_model_limits::normalize_model_list_patch(
        "allowed_models",
        payload.allowed_models,
    )?;
    payload.model_blacklist = crate::server::token_model_limits::normalize_model_list_patch(
        "model_blacklist",
        payload.model_blacklist,
    )?;

    if app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, &id)
        .await?
        .is_none()
    {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            "/me/tokens/{id}",
            "me_tokens_update",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    if payload.allowed_models.is_some() || payload.model_blacklist.is_some() {
        let current = app_state
            .token_store
            .get_token_by_id_scoped(&claims.sub, &id)
            .await?
            .ok_or_else(|| GatewayError::NotFound("token not found".into()))?;
        let mut next_allowed = current.allowed_models;
        let mut next_blacklist = current.model_blacklist;
        if let Some(v) = payload.allowed_models.as_ref() {
            next_allowed = v.clone();
        }
        if let Some(v) = payload.model_blacklist.as_ref() {
            next_blacklist = v.clone();
        }
        crate::server::token_model_limits::ensure_model_lists_mutually_exclusive(
            &next_allowed,
            &next_blacklist,
        )?;
    }
    if let Some(Some(list)) = payload.allowed_models.as_ref() {
        crate::server::token_model_limits::validate_models_exist_in_cache(
            &app_state,
            "allowed_models",
            &Some(list.clone()),
        )
        .await?;
    }
    if let Some(Some(list)) = payload.model_blacklist.as_ref() {
        crate::server::token_model_limits::validate_models_exist_in_cache(
            &app_state,
            "model_blacklist",
            &Some(list.clone()),
        )
        .await?;
    }

    let Some(updated) = app_state
        .token_store
        .update_token_by_id(&id, payload)
        .await?
    else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            "/me/tokens/{id}",
            "me_tokens_update",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    };

    log_simple_request(
        &app_state,
        start_time,
        "PUT",
        "/me/tokens/{id}",
        "me_tokens_update",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(MyTokenOut::from(updated)))
}

pub async fn delete_my_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "DELETE",
                "/me/tokens/{id}",
                "me_tokens_delete",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    if app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, &id)
        .await?
        .is_none()
    {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            "/me/tokens/{id}",
            "me_tokens_delete",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let deleted = app_state.token_store.delete_token_by_id(&id).await?;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            "/me/tokens/{id}",
            "me_tokens_delete",
            None,
            None,
            token_for_log(provided.as_deref()),
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
            "/me/tokens/{id}",
            "me_tokens_delete",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        Err(ge)
    }
}

#[derive(Debug, Deserialize)]
pub struct TogglePayload {
    pub enabled: bool,
}

pub async fn toggle_my_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<TogglePayload>,
) -> Result<Json<MyTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/me/tokens/{id}/toggle",
                "me_tokens_toggle",
                None,
                None,
                provided.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    if app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, &id)
        .await?
        .is_none()
    {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/me/tokens/{id}/toggle",
            "me_tokens_toggle",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let ok = app_state
        .token_store
        .set_enabled_by_id(&id, payload.enabled)
        .await?;
    if !ok {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/me/tokens/{id}/toggle",
            "me_tokens_toggle",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let Some(t) = app_state
        .token_store
        .get_token_by_id_scoped(&claims.sub, &id)
        .await?
    else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/me/tokens/{id}/toggle",
            "me_tokens_toggle",
            None,
            None,
            provided.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    };

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/me/tokens/{id}/toggle",
        "me_tokens_toggle",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(MyTokenOut::from(t)))
}
