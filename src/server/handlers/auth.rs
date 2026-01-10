use axum::http::HeaderMap;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64_URL_SAFE_NO_PAD;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::login::SessionEntry;
use crate::server::storage_traits::TuiSessionRecord;
use crate::users::UserRole;

pub const SESSION_COOKIE: &str = "gw_session";

#[allow(dead_code)]
pub enum AdminIdentity {
    Jwt(AccessTokenClaims),
    TuiSession(TuiSessionRecord),
    WebSession(SessionEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    pub sub: String,
    pub email: String,
    pub role: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub jti: Option<String>,
    pub exp: i64,
    #[serde(default)]
    pub iat: Option<i64>,
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        let trimmed = part.trim();
        let (key, value) = trimmed.split_once('=')?;
        if key.trim() == name {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn jwt_secret_optional() -> Option<Vec<u8>> {
    std::env::var("GW_JWT_SECRET")
        .ok()
        .map(|s| s.into_bytes())
        .filter(|s| !s.is_empty())
}

fn jwt_secret_required() -> Result<Vec<u8>, GatewayError> {
    jwt_secret_optional().ok_or_else(|| {
        GatewayError::Config(
            "JWT secret not configured (set env `GW_JWT_SECRET` to enable admin AccessToken)"
                .into(),
        )
    })
}

pub fn jwt_ttl_secs() -> u64 {
    std::env::var("GW_JWT_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8 * 60 * 60)
}

#[derive(Serialize)]
struct JwtHeader<'a> {
    alg: &'a str,
    typ: &'a str,
}

fn issue_access_token_with_secret(
    claims: &AccessTokenClaims,
    secret: &[u8],
) -> Result<String, GatewayError> {
    let header = JwtHeader {
        alg: "HS256",
        typ: "JWT",
    };
    let header_json = serde_json::to_vec(&header)?;
    let claims_json = serde_json::to_vec(claims)?;

    let header_b64 = B64_URL_SAFE_NO_PAD.encode(header_json);
    let claims_b64 = B64_URL_SAFE_NO_PAD.encode(claims_json);
    let signing_input = format!("{}.{}", header_b64, claims_b64);

    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|_| GatewayError::Config("invalid JWT secret".into()))?;
    mac.update(signing_input.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = B64_URL_SAFE_NO_PAD.encode(sig);

    Ok(format!("{}.{}", signing_input, sig_b64))
}

pub fn issue_access_token(claims: &AccessTokenClaims) -> Result<String, GatewayError> {
    let secret = jwt_secret_required()?;
    issue_access_token_with_secret(claims, &secret)
}

fn validate_access_token_with_secret(
    token: &str,
    secret: &[u8],
) -> Result<AccessTokenClaims, GatewayError> {
    let mut it = token.split('.');
    let Some(header_b64) = it.next() else {
        return Err(GatewayError::Unauthorized("invalid token".into()));
    };
    let Some(claims_b64) = it.next() else {
        return Err(GatewayError::Unauthorized("invalid token".into()));
    };
    let Some(sig_b64) = it.next() else {
        return Err(GatewayError::Unauthorized("invalid token".into()));
    };
    if it.next().is_some() {
        return Err(GatewayError::Unauthorized("invalid token".into()));
    }

    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let sig_bytes = B64_URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_| GatewayError::Unauthorized("invalid token".into()))?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|_| GatewayError::Config("invalid JWT secret".into()))?;
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&sig_bytes)
        .map_err(|_| GatewayError::Unauthorized("invalid token".into()))?;

    let claims_json = B64_URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| GatewayError::Unauthorized("invalid token".into()))?;
    let claims: AccessTokenClaims = serde_json::from_slice(&claims_json)
        .map_err(|_| GatewayError::Unauthorized("invalid token".into()))?;

    let now = Utc::now().timestamp();
    if claims.exp <= now {
        return Err(GatewayError::Unauthorized("token expired".into()));
    }

    Ok(claims)
}

pub fn validate_access_token(token: &str) -> Result<AccessTokenClaims, GatewayError> {
    let secret = jwt_secret_required()?;
    validate_access_token_with_secret(token, &secret)
}

pub fn ensure_access_token(headers: &HeaderMap) -> Result<AccessTokenClaims, GatewayError> {
    let Some(tok) = bearer_token(headers) else {
        return Err(GatewayError::Unauthorized("missing bearer token".into()));
    };
    validate_access_token(&tok)
}

fn role_allows_superadmin(role: &str) -> bool {
    matches!(UserRole::parse(role), Some(UserRole::Superadmin))
}

pub fn require_user(headers: &HeaderMap) -> Result<AccessTokenClaims, GatewayError> {
    ensure_access_token(headers)
}

pub async fn require_superadmin(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<AdminIdentity, GatewayError> {
    if let Some(token) = bearer_token(headers) {
        if token.split('.').count() == 3
            && let Some(secret) = jwt_secret_optional()
        {
            let claims = validate_access_token_with_secret(&token, &secret)?;
            if !role_allows_superadmin(&claims.role) {
                return Err(GatewayError::Forbidden("permission denied".into()));
            }
            return Ok(AdminIdentity::Jwt(claims));
        }

        if let Some(session) = app_state.login_manager.validate_tui_token(&token).await? {
            return Ok(AdminIdentity::TuiSession(session));
        }
    }

    if let Some(session_id) = cookie_value(headers, SESSION_COOKIE)
        && let Some(session) = app_state.login_manager.get_session(&session_id).await?
    {
        return Ok(AdminIdentity::WebSession(session));
    }

    Err(GatewayError::Unauthorized("管理员身份认证失败".into()))
}

pub async fn ensure_admin(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<AdminIdentity, GatewayError> {
    require_superadmin(headers, app_state).await
}

// 校验 Client Token（外部调用 `/v1/*` 的 API Token）：
// - 必须存在于 token_store
// - 必须启用、未过期、未超额
pub async fn ensure_client_token(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<String, GatewayError> {
    let provided = bearer_token(headers);
    let Some(tok) = provided else {
        return Err(GatewayError::Unauthorized("missing bearer token".into()));
    };
    let Some(t) = app_state.token_store.get_token(&tok).await? else {
        return Err(GatewayError::Unauthorized("invalid token".into()));
    };
    if !t.enabled {
        if let Some(max_amount) = t.max_amount
            && let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&tok)
                .await
            && spent >= max_amount
        {
            return Err(GatewayError::Unauthorized("token budget exceeded".into()));
        }
        return Err(GatewayError::Unauthorized("token disabled".into()));
    }
    if let Some(exp) = t.expires_at
        && chrono::Utc::now() > exp
    {
        return Err(GatewayError::Unauthorized("token expired".into()));
    }
    Ok(tok)
}
