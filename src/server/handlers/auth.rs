use axum::http::HeaderMap;

use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::login::SessionEntry;
use crate::server::storage_traits::TuiSessionRecord;

pub const SESSION_COOKIE: &str = "gw_session";

pub enum AdminIdentity {
    TuiSession(TuiSessionRecord),
    WebSession(SessionEntry),
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

pub async fn ensure_admin(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<AdminIdentity, GatewayError> {
    if let Some(token) = bearer_token(headers) {
        if let Some(session) = app_state.login_manager.validate_tui_token(&token).await? {
            return Ok(AdminIdentity::TuiSession(session));
        }
    }

    if let Some(session_id) = cookie_value(headers, SESSION_COOKIE) {
        if let Some(session) = app_state.login_manager.get_session(&session_id).await? {
            return Ok(AdminIdentity::WebSession(session));
        }
    }

    Err(GatewayError::Config("管理员认证失败".into()))
}

// 校验任意有效令牌：
// - 管理员“身份令牌”直接放行
// - 其他管理令牌需存在且启用、未过期
pub async fn ensure_client(
    headers: &HeaderMap,
    app_state: &AppState,
) -> Result<String, GatewayError> {
    let provided = bearer_token(headers);
    let Some(tok) = provided else {
        return Err(GatewayError::Config("missing bearer token".into()));
    };
    let Some(t) = app_state.token_store.get_token(&tok).await? else {
        return Err(GatewayError::Config("invalid token".into()));
    };
    if !t.enabled {
        if let Some(max_amount) = t.max_amount {
            if let Ok(spent) = app_state
                .log_store
                .sum_spent_amount_by_client_token(&tok)
                .await
            {
                if spent >= max_amount {
                    return Err(GatewayError::Config("token budget exceeded".into()));
                }
            }
        }
        return Err(GatewayError::Config("token disabled".into()));
    }
    if let Some(exp) = t.expires_at {
        if chrono::Utc::now() > exp {
            return Err(GatewayError::Config("token expired".into()));
        }
    }
    Ok(tok)
}
