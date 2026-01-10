use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::auth::{AccessTokenClaims, ensure_access_token, issue_access_token, jwt_ttl_secs};
use crate::error::{GatewayError, Result as AppResult};
use crate::refresh_tokens::{
    RefreshTokenRecord, hash_refresh_token, issue_refresh_token, refresh_ttl_secs,
};
use crate::server::AppState;
use crate::users::{CreateUserPayload, UpdateUserPayload, UserRole, UserStatus, verify_password};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub bootstrap_code: String,
    #[serde(flatten)]
    pub payload: CreateUserPayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub role: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: String,
    pub refresh_expires_at: String,
    pub user: AuthUser,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub expires_at: String,
    pub user: AuthUser,
}

fn env_optional(name: &'static str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn default_permissions_for_role(role: Option<UserRole>) -> Vec<String> {
    match role {
        Some(UserRole::Superadmin) => vec!["admin:*".into()],
        Some(UserRole::Admin) => vec!["admin:*".into()],
        Some(UserRole::Manager) => vec!["admin:read".into()],
        Some(UserRole::Cashier) => vec!["admin:read".into()],
        None => vec![],
    }
}

fn permissions_from_env_or_default(role: Option<UserRole>) -> Vec<String> {
    if let Some(raw) = env_optional("GW_ADMIN_PERMISSIONS") {
        let v: Vec<String> = raw
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    default_permissions_for_role(role)
}

fn claims_to_user(claims: &AccessTokenClaims) -> AuthUser {
    AuthUser {
        id: claims.sub.clone(),
        email: claims.email.clone(),
        role: claims.role.clone(),
        permissions: claims.permissions.clone(),
    }
}

pub async fn login(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Json<LoginResponse>> {
    let Some(user) = app_state
        .user_store
        .get_auth_by_email(payload.email.trim())
        .await?
    else {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    };
    let Some(password_hash) = user.password_hash.as_deref() else {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    };
    if !verify_password(payload.password.as_str(), password_hash)? {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    }

    let now = Utc::now();
    let exp = now + Duration::seconds(jwt_ttl_secs() as i64);
    let role = user.role.as_str().to_string();
    let mut claims = AccessTokenClaims {
        sub: user.id,
        email: user.email,
        permissions: permissions_from_env_or_default(Some(user.role)),
        role,
        jti: Some(Uuid::new_v4().to_string()),
        exp: exp.timestamp(),
        iat: Some(now.timestamp()),
    };

    if claims.permissions.is_empty() {
        claims.permissions = vec!["admin:read".into()];
    }

    let token = issue_access_token(&claims)?;

    let refresh_token = issue_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token);
    let refresh_exp = now + Duration::seconds(refresh_ttl_secs());
    app_state
        .refresh_token_store
        .create_refresh_token(RefreshTokenRecord {
            id: Uuid::new_v4().to_string(),
            user_id: claims.sub.clone(),
            token_hash: refresh_hash,
            created_at: now,
            expires_at: refresh_exp,
            revoked_at: None,
            replaced_by_id: None,
            last_used_at: None,
        })
        .await?;

    Ok(Json(LoginResponse {
        access_token: token,
        refresh_token,
        expires_at: exp.to_rfc3339(),
        refresh_expires_at: refresh_exp.to_rfc3339(),
        user: claims_to_user(&claims),
    }))
}

pub async fn me(headers: HeaderMap) -> AppResult<Json<MeResponse>> {
    let claims = ensure_access_token(&headers)?;
    let exp = Utc
        .timestamp_opt(claims.exp, 0)
        .single()
        .unwrap_or_else(Utc::now);
    Ok(Json(MeResponse {
        expires_at: exp.to_rfc3339(),
        user: claims_to_user(&claims),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ChangePasswordRequest>,
) -> AppResult<axum::http::StatusCode> {
    let claims = ensure_access_token(&headers)?;

    let old_password = payload.old_password.trim();
    if old_password.is_empty() {
        return Err(GatewayError::Config("old_password 不能为空".into()));
    }
    let new_password = payload.new_password.trim();
    if new_password.len() < 7 {
        return Err(GatewayError::Config(
            "new_password must be at least 7 characters long".into(),
        ));
    }

    let Some(user) = app_state
        .user_store
        .get_auth_by_email(claims.email.trim())
        .await?
    else {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    };
    if user.id != claims.sub {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    }
    let Some(password_hash) = user.password_hash.as_deref() else {
        return Err(GatewayError::Config("password not set".into()));
    };
    if !verify_password(old_password, password_hash)? {
        return Err(GatewayError::Config("invalid old_password".into()));
    }

    let updated = app_state
        .user_store
        .update_user(
            &user.id,
            UpdateUserPayload {
                first_name: None,
                last_name: None,
                username: None,
                email: None,
                phone_number: None,
                password: Some(new_password.to_string()),
                status: None,
                role: None,
            },
        )
        .await?;
    if updated.is_none() {
        return Err(GatewayError::NotFound("user not found".into()));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResponse {
    pub user: AuthUser,
}

pub async fn register(
    State(app_state): State<Arc<AppState>>,
    Json(mut req): Json<RegisterRequest>,
) -> AppResult<(axum::http::StatusCode, Json<RegisterResponse>)> {
    let expected = std::env::var("GATEWAY_BOOTSTRAP_CODE")
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| GatewayError::Unauthorized("registration is disabled".into()))?;
    if req.bootstrap_code.trim() != expected.trim() {
        return Err(GatewayError::Unauthorized("invalid bootstrap code".into()));
    }

    if app_state.user_store.any_users().await? {
        return Err(GatewayError::Forbidden(
            "registration is only allowed when there are no users".into(),
        ));
    }

    let password_len = req
        .payload
        .password
        .as_deref()
        .map(|s| s.trim().len())
        .unwrap_or(0);
    if password_len < 7 {
        return Err(GatewayError::Config(
            "password must be at least 7 characters long".into(),
        ));
    }

    req.payload.role = UserRole::Superadmin;
    req.payload.status = UserStatus::Active;

    let created = app_state.user_store.create_user(req.payload).await?;
    let role = created.role;
    let user = AuthUser {
        id: created.id,
        email: created.email,
        role: role.as_str().to_string(),
        permissions: permissions_from_env_or_default(Some(role)),
    };
    Ok((
        axum::http::StatusCode::CREATED,
        Json(RegisterResponse { user }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: String,
    pub refresh_expires_at: String,
}

pub async fn refresh(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<RefreshRequest>,
) -> AppResult<Json<RefreshResponse>> {
    let raw = payload.refresh_token.trim();
    if raw.is_empty() {
        return Err(GatewayError::Unauthorized("invalid refresh token".into()));
    }
    let token_hash = hash_refresh_token(raw);

    let now = Utc::now();
    let Some(stored) = app_state
        .refresh_token_store
        .get_refresh_token_by_hash(&token_hash)
        .await?
    else {
        return Err(GatewayError::Unauthorized("invalid refresh token".into()));
    };
    if stored.revoked_at.is_some() {
        return Err(GatewayError::Unauthorized("invalid refresh token".into()));
    }
    if stored.expires_at <= now {
        return Err(GatewayError::Unauthorized("refresh token expired".into()));
    }

    let revoked = app_state
        .refresh_token_store
        .revoke_refresh_token(&token_hash, now)
        .await?;
    if !revoked {
        return Err(GatewayError::Unauthorized("invalid refresh token".into()));
    }

    let Some(user) = app_state.user_store.get_user(&stored.user_id).await? else {
        return Err(GatewayError::Unauthorized("invalid refresh token".into()));
    };

    let exp = now + Duration::seconds(jwt_ttl_secs() as i64);
    let role = user.role.as_str().to_string();
    let mut claims = AccessTokenClaims {
        sub: user.id,
        email: user.email,
        permissions: permissions_from_env_or_default(Some(user.role)),
        role,
        jti: Some(Uuid::new_v4().to_string()),
        exp: exp.timestamp(),
        iat: Some(now.timestamp()),
    };
    if claims.permissions.is_empty() {
        claims.permissions = vec!["admin:read".into()];
    }
    let access_token = issue_access_token(&claims)?;

    let new_refresh_token = issue_refresh_token();
    let new_hash = hash_refresh_token(&new_refresh_token);
    let new_id = Uuid::new_v4().to_string();
    let refresh_exp = now + Duration::seconds(refresh_ttl_secs());
    app_state
        .refresh_token_store
        .create_refresh_token(RefreshTokenRecord {
            id: new_id.clone(),
            user_id: claims.sub.clone(),
            token_hash: new_hash,
            created_at: now,
            expires_at: refresh_exp,
            revoked_at: None,
            replaced_by_id: None,
            last_used_at: Some(now),
        })
        .await?;
    let _ = app_state
        .refresh_token_store
        .set_refresh_token_replaced_by(&token_hash, &new_id)
        .await;

    Ok(Json(RefreshResponse {
        access_token,
        refresh_token: new_refresh_token,
        expires_at: exp.to_rfc3339(),
        refresh_expires_at: refresh_exp.to_rfc3339(),
    }))
}
