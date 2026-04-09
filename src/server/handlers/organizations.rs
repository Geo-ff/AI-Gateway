use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::auth::require_superadmin;
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OrganizationOut {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationPayload {
    pub organization_id: String,
}

fn normalize_organization_id(raw: &str) -> Result<String, GatewayError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(GatewayError::Config("organization_id 不能为空".into()));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(GatewayError::Config(
            "organization_id 不能包含控制字符".into(),
        ));
    }
    if trimmed.chars().count() > 128 {
        return Err(GatewayError::Config(
            "organization_id 长度不能超过 128".into(),
        ));
    }
    Ok(trimmed.to_string())
}

pub async fn list_organizations(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<OrganizationOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/organizations",
            "organizations_list",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let organizations = app_state
        .organizations
        .list_organizations()
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .map(|id| OrganizationOut { id })
        .collect();

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/organizations",
        "organizations_list",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(organizations))
}

pub async fn create_organization(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrganizationPayload>,
) -> Result<(axum::http::StatusCode, Json<OrganizationOut>), GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = require_superadmin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/organizations",
            "organizations_create",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let organization_id = normalize_organization_id(&payload.organization_id)?;
    app_state
        .organizations
        .create_organization(&organization_id)
        .await
        .map_err(GatewayError::Db)?;

    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/organizations",
        "organizations_create",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        201,
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(OrganizationOut {
            id: organization_id,
        }),
    ))
}
