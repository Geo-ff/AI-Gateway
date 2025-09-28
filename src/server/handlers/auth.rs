use axum::http::HeaderMap;

use crate::error::GatewayError;
use crate::server::AppState;

pub fn ensure_admin(headers: &HeaderMap, app_state: &AppState) -> Result<(), GatewayError> {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    match provided {
        Some(tok) if tok == app_state.admin_identity_token => Ok(()),
        _ => Err(GatewayError::Config("admin token invalid".into())),
    }
}

// 校验任意有效令牌：
// - 管理员“身份令牌”直接放行
// - 其他管理令牌需存在且启用、未过期
pub async fn ensure_client(headers: &HeaderMap, app_state: &AppState) -> Result<bool, GatewayError> {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());
    let Some(tok) = provided else { return Err(GatewayError::Config("missing bearer token".into())); };
    if tok == app_state.admin_identity_token {
        return Ok(true);
    }
    if let Some(t) = app_state.token_store.get_token(&tok).await? {
        if !t.enabled {
            // 若因余额不足被禁用，返回更具体错误
            if let Some(max_amount) = t.max_amount {
                if let Ok(spent) = app_state.log_store.sum_spent_amount_by_client_token(&tok).await {
                    if spent >= max_amount { return Err(GatewayError::Config("token budget exceeded".into())); }
                }
            }
            return Err(GatewayError::Config("token disabled".into()));
        }
        if let Some(exp) = t.expires_at { if chrono::Utc::now() > exp { return Err(GatewayError::Config("token expired".into())); } }
        return Ok(false);
    }
    Err(GatewayError::Config("invalid token".into()))
}
