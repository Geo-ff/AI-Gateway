use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use super::auth::ensure_admin;
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::login::LoginManager;
use crate::server::storage_traits::AdminPublicKeyRecord;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use chrono::Utc;

#[derive(Debug, Serialize)]
pub struct AdminKeyOut {
    pub fingerprint: String,
    pub comment: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddKeyPayload {
    pub public_key_b64: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

pub async fn list_keys(
    State(app): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<AdminKeyOut>>, GatewayError> {
    ensure_admin(&headers, &app).await?;
    let keys = app.login_manager.list_admin_keys().await?;
    let out = keys
        .into_iter()
        .map(|k| AdminKeyOut {
            fingerprint: k.fingerprint,
            comment: k.comment,
            enabled: k.enabled,
            created_at: k.created_at.to_rfc3339(),
            last_used_at: k.last_used_at.map(|v| v.to_rfc3339()),
        })
        .collect();
    Ok(Json(out))
}

pub async fn add_key(
    State(app): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<AddKeyPayload>,
) -> Result<Json<AdminKeyOut>, GatewayError> {
    ensure_admin(&headers, &app).await?;
    let raw = B64_STANDARD
        .decode(payload.public_key_b64.as_bytes())
        .map_err(|_| GatewayError::Config("public_key_b64 无法解码".into()))?;
    if raw.len() != ed25519_dalek::PUBLIC_KEY_LENGTH {
        return Err(GatewayError::Config("公钥长度必须为 32 字节".into()));
    }
    let vk = VerifyingKey::from_bytes(
        &raw.clone()
            .try_into()
            .map_err(|_| GatewayError::Config("公钥长度不正确".into()))?,
    )
    .map_err(|_| GatewayError::Config("公钥解析失败".into()))?;
    let fp = LoginManager::fingerprint_for_public_key(&vk.to_bytes());
    let rec = AdminPublicKeyRecord {
        fingerprint: fp.clone(),
        public_key: raw,
        comment: payload.comment.clone(),
        enabled: payload.enabled.unwrap_or(true),
        created_at: Utc::now(),
        last_used_at: None,
    };
    app.login_manager.add_admin_key(&rec).await?;
    Ok(Json(AdminKeyOut {
        fingerprint: fp,
        comment: rec.comment,
        enabled: rec.enabled,
        created_at: rec.created_at.to_rfc3339(),
        last_used_at: None,
    }))
}

pub async fn delete_key(
    State(app): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Path(fingerprint): Path<String>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    ensure_admin(&headers, &app).await?;
    // 安全保护：禁止删除最后一把启用中的管理员密钥
    let keys = app.login_manager.list_admin_keys().await?;
    let enabled_count = keys.iter().filter(|k| k.enabled).count();
    let target = keys.iter().find(|k| k.fingerprint == fingerprint);
    if let Some(t) = target {
        if t.enabled && enabled_count <= 1 {
            return Err(GatewayError::Config(
                "不能删除最后一把启用的管理员密钥".into(),
            ));
        }
    } else {
        return Err(GatewayError::NotFound("fingerprint not found".into()));
    }

    let ok = app.login_manager.delete_admin_key(&fingerprint).await?;
    if ok {
        Ok(Json(serde_json::json!({"deleted": true})))
    } else {
        Err(GatewayError::NotFound("fingerprint not found".into()))
    }
}
