use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::auth::{AccessTokenClaims, ensure_access_token, issue_access_token, jwt_ttl_secs};
use crate::error::{GatewayError, Result as AppResult};
use crate::server::AppState;
use crate::users::{CreateUserPayload, UserRole, UserStatus, verify_password};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
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
    pub expires_at: String,
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

fn role_allows_admin(role: &str) -> bool {
    matches!(
        UserRole::parse(role),
        Some(UserRole::Admin | UserRole::Superadmin)
    )
}

pub async fn login(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<LoginRequest>,
) -> AppResult<Json<LoginResponse>> {
    let Some(user) = app_state.user_store.get_auth_by_email(payload.email.trim()).await? else {
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
        exp: exp.timestamp(),
        iat: Some(now.timestamp()),
    };

    if claims.permissions.is_empty() {
        claims.permissions = vec!["admin:read".into()];
    }

    let token = issue_access_token(&claims)?;
    Ok(Json(LoginResponse {
        access_token: token,
        expires_at: exp.to_rfc3339(),
        user: claims_to_user(&claims),
    }))
}

pub async fn me(headers: HeaderMap) -> AppResult<Json<MeResponse>> {
    let claims = ensure_access_token(&headers)?;
    if !role_allows_admin(&claims.role) {
        return Err(GatewayError::Forbidden("permission denied".into()));
    }
    let exp = Utc
        .timestamp_opt(claims.exp, 0)
        .single()
        .unwrap_or_else(Utc::now);
    Ok(Json(MeResponse {
        expires_at: exp.to_rfc3339(),
        user: claims_to_user(&claims),
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterResponse {
    pub user: AuthUser,
}

pub async fn register(
    State(app_state): State<Arc<AppState>>,
    Json(mut payload): Json<CreateUserPayload>,
) -> AppResult<(axum::http::StatusCode, Json<RegisterResponse>)> {
    if app_state.user_store.any_users().await? {
        return Err(GatewayError::Forbidden(
            "registration is only allowed when there are no users".into(),
        ));
    }

    let password_len = payload.password.as_deref().map(|s| s.trim().len()).unwrap_or(0);
    if password_len < 7 {
        return Err(GatewayError::Config(
            "password must be at least 7 characters long".into(),
        ));
    }

    payload.role = UserRole::Superadmin;
    payload.status = UserStatus::Active;

    let created = app_state.user_store.create_user(payload).await?;
    let role = created.role;
    let user = AuthUser {
        id: created.id,
        email: created.email,
        role: role.as_str().to_string(),
        permissions: permissions_from_env_or_default(Some(role)),
    };
    Ok((axum::http::StatusCode::CREATED, Json(RegisterResponse { user })))
}
