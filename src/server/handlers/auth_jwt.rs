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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
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

fn db_user_to_auth_user(claims: &AccessTokenClaims, user: crate::users::User) -> AuthUser {
    let name = {
        let first = user.first_name.trim();
        let last = user.last_name.trim();
        if first.is_empty() && last.is_empty() {
            None
        } else if last.is_empty() {
            Some(first.to_string())
        } else if first.is_empty() {
            Some(last.to_string())
        } else {
            Some(format!("{} {}", first, last))
        }
    };
    AuthUser {
        id: user.id,
        name,
        username: Some(user.username),
        bio: user.bio,
        theme: user.theme,
        font: user.font,
        email: user.email,
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
    let db_user = app_state
        .user_store
        .get_user(&claims.sub)
        .await?
        .ok_or_else(|| GatewayError::Unauthorized("invalid credentials".into()))?;

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
        user: db_user_to_auth_user(&claims, db_user),
    }))
}

pub async fn me(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> AppResult<Json<MeResponse>> {
    let claims = ensure_access_token(&headers)?;
    let db_user = app_state
        .user_store
        .get_user(&claims.sub)
        .await?
        .ok_or_else(|| GatewayError::Unauthorized("invalid credentials".into()))?;
    let exp = Utc
        .timestamp_opt(claims.exp, 0)
        .single()
        .unwrap_or_else(Utc::now);
    Ok(Json(MeResponse {
        expires_at: exp.to_rfc3339(),
        user: db_user_to_auth_user(&claims, db_user),
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchMeRequest {
    pub name: Option<String>,
    pub username: Option<String>,
    pub bio: Option<String>,
    pub theme: Option<String>,
    pub font: Option<String>,
}

fn validate_theme(theme: &str) -> bool {
    matches!(theme, "light" | "dark" | "system")
}

fn validate_font(font: &str) -> bool {
    matches!(font, "inter" | "manrope" | "system")
}

pub async fn patch_me(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PatchMeRequest>,
) -> AppResult<Json<AuthUser>> {
    let claims = ensure_access_token(&headers)?;

    let Some(existing) = app_state.user_store.get_user(&claims.sub).await? else {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    };

    if let Some(username) = payload.username.as_deref() {
        let username = username.trim();
        if username.is_empty() {
            return Err(GatewayError::Config("username 不能为空".into()));
        }
        if username != existing.username {
            if let Some(other) = app_state.user_store.get_user_by_username(username).await? {
                if other.id != claims.sub {
                    return Err(GatewayError::Config("username 已被占用".into()));
                }
            }
        }
    }
    if let Some(theme) = payload.theme.as_deref() {
        if !validate_theme(theme.trim()) {
            return Err(GatewayError::Config("theme 取值必须为 light/dark/system".into()));
        }
    }
    if let Some(font) = payload.font.as_deref() {
        if !validate_font(font.trim()) {
            return Err(GatewayError::Config(
                "font 取值必须为 inter/manrope/system".into(),
            ));
        }
    }

    let (first_name, last_name) = match payload.name.as_deref().map(str::trim) {
        Some(v) if !v.is_empty() => (Some(v.to_string()), Some(String::new())),
        Some(_) => return Err(GatewayError::Config("name 不能为空".into())),
        None => (None, None),
    };

    let updated = app_state
        .user_store
        .update_user(
            &claims.sub,
            UpdateUserPayload {
                first_name,
                last_name,
                username: payload.username.map(|v| v.trim().to_string()),
                bio: payload.bio.map(|v| v.trim().to_string()),
                theme: payload.theme.map(|v| v.trim().to_string()),
                font: payload.font.map(|v| v.trim().to_string()),
                email: None,
                phone_number: None,
                password: None,
                status: None,
                role: None,
            },
        )
        .await?;

    let updated = updated.ok_or_else(|| GatewayError::NotFound("user not found".into()))?;
    Ok(Json(db_user_to_auth_user(&claims, updated)))
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
        return Err(GatewayError::Config("new_password 长度至少 7 位".into()));
    }
    if new_password == old_password {
        return Err(GatewayError::Config("new_password 不能与 old_password 相同".into()));
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
        return Err(GatewayError::Config("旧密码不正确".into()));
    }

    let updated = app_state
        .user_store
        .update_user(
            &user.id,
            UpdateUserPayload {
                first_name: None,
                last_name: None,
                username: None,
                bio: None,
                theme: None,
                font: None,
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
        name: None,
        username: Some(created.username),
        bio: created.bio,
        theme: created.theme,
        font: created.font,
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
