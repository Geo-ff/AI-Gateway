use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result as AppResult};
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct ChallengeReq {
    pub fingerprint: String,
}

#[derive(Debug, Serialize)]
pub struct ChallengeResp {
    pub challenge_id: String,
    pub nonce: String,
    pub expires_at: String,
    pub alg: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct VerifyReq {
    pub challenge_id: String,
    pub fingerprint: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResp {
    pub token: String,
    pub expires_at: String,
    pub fingerprint: String,
}

pub async fn challenge(
    State(app): State<Arc<AppState>>,
    Json(payload): Json<ChallengeReq>,
) -> AppResult<Json<ChallengeResp>> {
    let fingerprint = payload.fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(GatewayError::Config("fingerprint required".into()));
    }
    let challenge = app.login_manager.issue_challenge(fingerprint).await?;
    Ok(Json(ChallengeResp {
        challenge_id: challenge.challenge_id,
        nonce: challenge.nonce_b64,
        expires_at: challenge.expires_at.to_rfc3339(),
        alg: "ed25519",
    }))
}

pub async fn verify(
    State(app): State<Arc<AppState>>,
    Json(payload): Json<VerifyReq>,
) -> AppResult<Json<VerifyResp>> {
    let fingerprint = payload.fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(GatewayError::Config("fingerprint required".into()));
    }
    if payload.challenge_id.trim().is_empty() {
        return Err(GatewayError::Config("challenge_id required".into()));
    }
    if payload.signature.trim().is_empty() {
        return Err(GatewayError::Config("signature required".into()));
    }
    let session = app
        .login_manager
        .verify_challenge(
            payload.challenge_id.trim(),
            fingerprint,
            payload.signature.trim(),
        )
        .await?;
    Ok(Json(VerifyResp {
        token: session.token,
        expires_at: session.expires_at.to_rfc3339(),
        fingerprint: session.fingerprint,
    }))
}
