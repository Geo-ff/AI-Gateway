use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use super::auth::ensure_admin;
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::storage_traits::TuiSessionRecord;

#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct TuiSessionOut {
    pub session_id: String,
    pub fingerprint: String,
    pub issued_at: String,
    pub expires_at: String,
    pub revoked: bool,
    pub last_code_at: Option<String>,
}

fn map_session(r: TuiSessionRecord) -> TuiSessionOut {
    TuiSessionOut {
        session_id: r.session_id,
        fingerprint: r.fingerprint,
        issued_at: r.issued_at.to_rfc3339(),
        expires_at: r.expires_at.to_rfc3339(),
        revoked: r.revoked,
        last_code_at: r.last_code_at.map(|v| v.to_rfc3339()),
    }
}

pub async fn list_tui_sessions(
    State(app): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<SessionQuery>,
) -> Result<Json<Vec<TuiSessionOut>>, GatewayError> {
    ensure_admin(&headers, &app).await?;
    let list = app
        .login_manager
        .list_tui_sessions(q.fingerprint.as_deref())
        .await?
        .into_iter()
        .map(map_session)
        .collect();
    Ok(Json(list))
}

pub async fn revoke_tui_session(
    State(app): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    ensure_admin(&headers, &app).await?;
    let ok = app.login_manager.revoke_tui_session(&session_id).await?;
    Ok(Json(serde_json::json!({"revoked": ok})))
}
