use axum::{Json, http::HeaderMap};
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::auth::{AccessTokenClaims, ensure_access_token, issue_access_token, jwt_ttl_secs};
use crate::error::{GatewayError, Result as AppResult};
use crate::users::UserRole;

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

fn env_required(name: &'static str) -> Result<String, GatewayError> {
    std::env::var(name).map_err(|_| GatewayError::Config(format!("missing env `{}`", name)))
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

pub async fn login(Json(payload): Json<LoginRequest>) -> AppResult<Json<LoginResponse>> {
    let expected_email = env_required("GW_ADMIN_EMAIL")?;
    let expected_password = env_required("GW_ADMIN_PASSWORD")?;

    if payload.email != expected_email || payload.password != expected_password {
        return Err(GatewayError::Unauthorized("invalid credentials".into()));
    }

    let id = env_optional("GW_ADMIN_ID").unwrap_or_else(|| "admin".into());
    let role = env_optional("GW_ADMIN_ROLE").unwrap_or_else(|| "admin".into());
    let parsed_role = UserRole::parse(&role);
    if parsed_role.is_none() {
        return Err(GatewayError::Config("invalid env `GW_ADMIN_ROLE`".into()));
    }

    let now = Utc::now();
    let exp = now + Duration::seconds(jwt_ttl_secs() as i64);
    let mut claims = AccessTokenClaims {
        sub: id,
        email: expected_email,
        role,
        permissions: permissions_from_env_or_default(parsed_role),
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
