use std::sync::Arc;

use axum::{extract::State, http::{HeaderMap, HeaderValue, StatusCode}, Json, response::IntoResponse};
use chrono::{Utc};
use serde::{Deserialize, Serialize};

use crate::{error::{GatewayError, Result as AppResult}, server::{AppState, login::LoginCodeEntry}};
use super::auth::ensure_admin;

#[derive(Debug, Deserialize)]
pub struct CreateCodePayload {
    #[serde(default = "default_ttl")] pub ttl_secs: u64,
    #[serde(default = "default_max_uses")] pub max_uses: u32,
    #[serde(default = "default_len")] pub length: usize,
    #[serde(default)] pub magic_url: bool,
}

fn default_ttl() -> u64 { 60 }
fn default_max_uses() -> u32 { 1 }
fn default_len() -> usize { 40 }

#[derive(Debug, Serialize)]
pub struct CreateCodeResponse {
    pub code: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub login_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RedeemPayload { pub code: String }

#[derive(Debug, Serialize)]
pub struct SessionInfo { pub valid: bool, #[serde(skip_serializing_if = "Option::is_none")] pub user: Option<UserInfo> }

#[derive(Debug, Serialize)]
pub struct UserInfo { pub name: String }

const SESSION_COOKIE: &str = "gw_session";

fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') { let kv = part.trim(); if let Some((k, v)) = kv.split_once('=') { if k.trim() == name { return Some(v.trim().to_string()); } } }
    None
}

fn set_session_cookie(session_id: &str, secure: bool) -> HeaderValue {
    // For dev, we use SameSite=Lax to allow on same-origin (e.g., via Vite proxy)
    let mut v = format!("{}={}; Path=/; HttpOnly; SameSite=Lax", SESSION_COOKIE, session_id);
    if secure { v.push_str("; Secure"); }
    HeaderValue::from_str(&v).unwrap_or(HeaderValue::from_static(""))
}

fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_static("gw_session=deleted; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
}

fn is_secure(headers: &HeaderMap) -> bool {
    headers.get("X-Forwarded-Proto").and_then(|v| v.to_str().ok()).map(|s| s.eq_ignore_ascii_case("https")).unwrap_or(false)
}

pub async fn create_login_code(State(app): State<Arc<AppState>>, headers: HeaderMap, Json(payload): Json<CreateCodePayload>) -> AppResult<Json<CreateCodeResponse>> {
    // 暂以 admin identity token 授权生成（后续切换为 TUI 会话）
    ensure_admin(&headers, &app)?;
    if payload.length < 25 || payload.length > 64 { return Err(GatewayError::Config("code length must be 25..=64".into())); }
    let ttl = payload.ttl_secs.max(1).min(24 * 60 * 60);
    let max_uses = payload.max_uses.max(1).min(1000);
    tracing::info!(
        ttl_secs = ttl,
        max_uses = max_uses,
        length = payload.length,
        magic_url = payload.magic_url,
        "create_login_code request"
    );
    let entry: LoginCodeEntry = app.login_manager.generate_code(ttl, max_uses, payload.length).await;

    // 使用 Hash 路由，避免 dev 代理与 SPA 冲突（/#/auth/magic?code=...）
    let login_url = if payload.magic_url { Some(format!("/#/auth/magic?code={}", entry.code)) } else { None };
    tracing::debug!(
        expires_at = %entry.expires_at,
        // 不打印完整 code，避免泄露
        code_preview = %format!("{}…{}", &entry.code.chars().take(3).collect::<String>(), &entry.code.chars().rev().take(3).collect::<String>().chars().rev().collect::<String>()),
        "login code created"
    );
    Ok(Json(CreateCodeResponse { code: entry.code, expires_at: entry.expires_at.to_rfc3339(), login_url }))
}

pub async fn redeem_code(State(app): State<Arc<AppState>>, headers: HeaderMap, Json(payload): Json<RedeemPayload>) -> AppResult<impl IntoResponse> {
    tracing::info!(
        code_preview = %payload.code.get(0..3).unwrap_or("").to_string(),
        "attempt redeem code"
    );
    let Some(sess) = app.login_manager.redeem(&payload.code).await else {
        tracing::warn!("redeem failed: invalid/expired/used");
        return Err(GatewayError::Config("invalid or expired code".into()));
    };
    let mut resp = axum::response::Response::builder().status(StatusCode::NO_CONTENT).body(axum::body::Body::empty()).unwrap();
    let secure = is_secure(&headers);
    resp.headers_mut().insert(axum::http::header::SET_COOKIE, set_session_cookie(&sess.id, secure));
    tracing::info!(secure_cookie = secure, session_id_preview = %sess.id.get(0..6).unwrap_or(""), "redeem success, session issued");
    Ok(resp)
}

pub async fn get_session(State(app): State<Arc<AppState>>, headers: HeaderMap) -> AppResult<Json<SessionInfo>> {
    let sid = parse_cookie(&headers, SESSION_COOKIE);
    if let Some(id) = sid {
        let ok = app.login_manager.get_session(&id).await.is_some();
        tracing::debug!(have_cookie = true, valid = ok, "get_session check");
        if ok { return Ok(Json(SessionInfo { valid: true, user: Some(UserInfo { name: "admin".into() }) })); }
    } else {
        tracing::debug!(have_cookie = false, "get_session no cookie");
    }
    Ok(Json(SessionInfo { valid: false, user: None }))
}

pub async fn logout(State(app): State<Arc<AppState>>, headers: HeaderMap) -> AppResult<impl IntoResponse> {
    if let Some(id) = parse_cookie(&headers, SESSION_COOKIE) { let _ = app.login_manager.revoke_session(&id).await; tracing::info!("logout: session revoked"); } else { tracing::info!("logout: no session cookie"); }
    let mut resp = axum::response::Response::builder().status(StatusCode::NO_CONTENT).body(axum::body::Body::empty()).unwrap();
    resp.headers_mut().insert(axum::http::header::SET_COOKIE, clear_session_cookie());
    Ok(resp)
}
