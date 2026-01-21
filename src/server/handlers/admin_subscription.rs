use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use std::sync::Arc;

use super::auth::{AdminIdentity, require_superadmin};
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};
use crate::subscription::SubscriptionPlan;

fn identity_updated_by(identity: &AdminIdentity) -> Option<String> {
    match identity {
        AdminIdentity::Jwt(claims) => Some(claims.sub.clone()),
        AdminIdentity::TuiSession(s) => Some(s.fingerprint.clone()),
        AdminIdentity::WebSession(s) => s.fingerprint.clone(),
    }
}

pub async fn get_draft_plans(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/subscription/plans/draft",
            "admin_subscription_draft_get",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let rec = app_state.subscription_store.get_draft_plans().await?;
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/subscription/plans/draft",
        "admin_subscription_draft_get",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "scope": rec.scope,
        "updated_at": crate::logging::time::to_iso8601_utc_string(&rec.updated_at),
        "updated_by": rec.updated_by,
        "plans": rec.plans,
    })))
}

pub async fn put_draft_plans(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(plans): Json<Vec<SubscriptionPlan>>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let identity = match require_superadmin(&headers, &app_state).await {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                "/admin/subscription/plans/draft",
                "admin_subscription_draft_put",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let rec = app_state
        .subscription_store
        .put_draft_plans(plans, identity_updated_by(&identity))
        .await?;

    log_simple_request(
        &app_state,
        start_time,
        "PUT",
        "/admin/subscription/plans/draft",
        "admin_subscription_draft_put",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "scope": rec.scope,
        "updated_at": crate::logging::time::to_iso8601_utc_string(&rec.updated_at),
        "updated_by": rec.updated_by,
        "plans": rec.plans,
    })))
}

pub async fn publish_draft(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    let identity = match require_superadmin(&headers, &app_state).await {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "POST",
                "/admin/subscription/plans/publish",
                "admin_subscription_publish",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let rec = app_state
        .subscription_store
        .publish_draft(identity_updated_by(&identity))
        .await?;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/subscription/plans/publish",
        "admin_subscription_publish",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(serde_json::json!({
        "scope": rec.scope,
        "updated_at": crate::logging::time::to_iso8601_utc_string(&rec.updated_at),
        "updated_by": rec.updated_by,
        "plans": rec.plans,
    })))
}
