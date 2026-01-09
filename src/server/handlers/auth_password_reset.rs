use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use chrono::{Duration, Utc};
use resend_rs::{Resend, types::CreateEmailBaseOptions};
use serde::Deserialize;
use uuid::Uuid;

use crate::error::{GatewayError, Result as AppResult};
use crate::password_reset_tokens::{
    hash_password_reset_token, issue_password_reset_token, PasswordResetTokenRecord,
};
use crate::server::AppState;
use crate::users::UpdateUserPayload;

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    pub token: String,
    pub new_password: String,
}

fn env_non_empty(name: &'static str) -> Option<String> {
    std::env::var(name).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn reset_password_ttl_secs() -> i64 {
    std::env::var("GW_RESET_PASSWORD_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30 * 60)
}

fn forgot_password_min_interval_secs() -> i64 {
    std::env::var("GW_FORGOT_PASSWORD_MIN_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60)
}

fn reset_password_path() -> String {
    env_non_empty("RESET_PASSWORD_PATH").unwrap_or_else(|| "/reset-password".into())
}

fn join_base_and_path(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let p = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    format!("{}{}", base, p)
}

fn resend_ready() -> bool {
    env_non_empty("RESEND_API_KEY").is_some()
        && env_non_empty("RESEND_FROM").is_some()
        && env_non_empty("CAPTOK_BASE_URL").is_some()
}

fn build_reset_link(token: &str) -> Option<String> {
    let base_url = env_non_empty("CAPTOK_BASE_URL")?;
    let url = join_base_and_path(&base_url, reset_password_path().as_str());
    Some(format!("{}?token={}", url, token))
}

async fn maybe_send_reset_email(to: &str, token: &str) {
    if !resend_ready() {
        tracing::warn!(
            "RESEND_API_KEY/RESEND_FROM/CAPTOK_BASE_URL not configured; password reset email not sent"
        );
        return;
    }

    let Some(from) = env_non_empty("RESEND_FROM") else {
        tracing::warn!("RESEND_FROM not configured; password reset email not sent");
        return;
    };
    let Some(link) = build_reset_link(token) else {
        tracing::warn!("CAPTOK_BASE_URL not configured; password reset email not sent");
        return;
    };

    let resend = Resend::default();
    let subject = "Reset your password";
    let html = format!(
        "<p>Click the link below to reset your password:</p>\
         <p><a href=\"{link}\">{link}</a></p>\
         <p>If you did not request this, you can ignore this email.</p>",
    );

    let email = CreateEmailBaseOptions::new(from, [to.to_string()], subject).with_html(&html);
    if let Err(e) = resend.emails.send(email).await {
        tracing::warn!("failed to send password reset email: {}", e);
    }
}

pub async fn forgot_password(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<ForgotPasswordRequest>,
) -> AppResult<StatusCode> {
    let email = payload.email.trim();
    if email.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }

    let user = match app_state.user_store.get_auth_by_email(email).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("forgot-password lookup failed: {}", e);
            return Ok(StatusCode::NO_CONTENT);
        }
    };
    let Some(user) = user else {
        return Ok(StatusCode::NO_CONTENT);
    };

    if !resend_ready() {
        tracing::warn!(
            user_id = %user.id,
            "password reset requested but Resend env not configured"
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let now = Utc::now();
    let since = now - Duration::seconds(forgot_password_min_interval_secs());
    let rate_limited = match app_state
        .password_reset_token_store
        .has_recent_active_password_reset_token(&user.id, since, now)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("forgot-password rate-limit check failed: {}", e);
            return Ok(StatusCode::NO_CONTENT);
        }
    };
    if rate_limited {
        tracing::info!(user_id = %user.id, "password reset request rate-limited");
        return Ok(StatusCode::NO_CONTENT);
    }

    let token = issue_password_reset_token();
    let token_hash = hash_password_reset_token(&token);
    let exp = now + Duration::seconds(reset_password_ttl_secs());
    if let Err(e) = app_state
        .password_reset_token_store
        .create_password_reset_token(PasswordResetTokenRecord {
            id: Uuid::new_v4().to_string(),
            user_id: user.id.clone(),
            token_hash,
            created_at: now,
            expires_at: exp,
            used_at: None,
        })
        .await
    {
        tracing::error!("forgot-password token insert failed: {}", e);
        return Ok(StatusCode::NO_CONTENT);
    }

    tracing::info!(user_id = %user.id, "password reset token created");
    maybe_send_reset_email(user.email.as_str(), &token).await;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_password(
    State(app_state): State<Arc<AppState>>,
    Json(payload): Json<ResetPasswordRequest>,
) -> AppResult<StatusCode> {
    let new_password = payload.new_password.trim();
    if new_password.len() < 7 {
        return Err(GatewayError::Config(
            "newPassword must be at least 7 characters long".into(),
        ));
    }

    let raw = payload.token.trim();
    if raw.is_empty() {
        return Err(GatewayError::Unauthorized("invalid reset token".into()));
    }
    let token_hash = hash_password_reset_token(raw);

    let now = Utc::now();
    let Some(token) = app_state
        .password_reset_token_store
        .consume_password_reset_token(&token_hash, now)
        .await?
    else {
        return Err(GatewayError::Unauthorized("invalid reset token".into()));
    };

    let updated = app_state
        .user_store
        .update_user(
            &token.user_id,
            UpdateUserPayload {
                first_name: None,
                last_name: None,
                username: None,
                email: None,
                phone_number: None,
                password: Some(new_password.to_string()),
                status: None,
                role: None,
            },
        )
        .await?;
    if updated.is_none() {
        return Err(GatewayError::Unauthorized("invalid reset token".into()));
    }

    let _ = app_state
        .refresh_token_store
        .revoke_all_refresh_tokens_for_user(&token.user_id, now)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
