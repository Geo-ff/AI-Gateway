use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::server::util::{bearer_token, token_for_log};
use crate::{
    admin::{AdminToken, CreateTokenPayload, UpdateTokenPayload},
    error::GatewayError,
    server::AppState,
};

#[derive(Debug, Serialize)]
pub struct AdminTokenOut {
    pub id: String,
    pub name: String,
    pub token: String,
    pub allowed_models: Option<Vec<String>>,
    pub max_tokens: Option<i64>,
    pub max_amount: Option<f64>,
    pub amount_spent: f64,
    pub prompt_tokens_spent: i64,
    pub completion_tokens_spent: i64,
    pub total_tokens_spent: i64,
    pub usage_count: i64,
    pub enabled: bool,
    pub expires_at: Option<String>, // 以北京时间字符串返回
    pub created_at: String,
}

impl From<AdminToken> for AdminTokenOut {
    fn from(t: AdminToken) -> Self {
        Self {
            id: t.id,
            name: t.name,
            token: t.token,
            allowed_models: t.allowed_models,
            max_tokens: t.max_tokens,
            max_amount: t.max_amount,
            amount_spent: t.amount_spent,
            prompt_tokens_spent: t.prompt_tokens_spent,
            completion_tokens_spent: t.completion_tokens_spent,
            total_tokens_spent: t.total_tokens_spent,
            usage_count: 0,
            enabled: t.enabled,
            expires_at: t
                .expires_at
                .as_ref()
                .map(crate::logging::time::to_beijing_string),
            created_at: crate::logging::time::to_beijing_string(&t.created_at),
        }
    }
}

use super::auth::ensure_admin;
use crate::server::request_logging::log_simple_request;
use chrono::Utc;

fn validate_admin_token_name(name: &str) -> Result<String, GatewayError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(GatewayError::Config("name 不能为空".into()));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(GatewayError::Config("name 不能包含控制字符".into()));
    }
    if trimmed.chars().count() > 64 {
        return Err(GatewayError::Config("name 长度不能超过 64".into()));
    }
    Ok(trimmed.to_string())
}

pub async fn list_tokens(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminTokenOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/tokens",
            "admin_tokens_list",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    use std::collections::HashMap;
    let usage_counts: HashMap<String, i64> = app_state
        .log_store
        .count_requests_by_client_token()
        .await
        .map_err(GatewayError::Db)?
        .into_iter()
        .collect();
    let tokens = app_state
        .token_store
        .list_tokens()
        .await?
        .into_iter()
        .map(|token| {
            let mut out = AdminTokenOut::from(token.clone());
            if let Some(count) = usage_counts.get(&token.token) {
                out.usage_count = *count;
            }
            out
        })
        .collect();
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/tokens",
        "admin_tokens_list",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(tokens))
}

pub async fn get_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/tokens/{id}",
            "admin_tokens_get",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    match app_state.token_store.get_token_by_id(&id).await? {
        Some(t) => {
            let mut out = AdminTokenOut::from(t.clone());
            let usage_counts: std::collections::HashMap<String, i64> = app_state
                .log_store
                .count_requests_by_client_token()
                .await
                .map_err(GatewayError::Db)?
                .into_iter()
                .collect();
            out.usage_count = usage_counts.get(&t.token).copied().unwrap_or(0);
            Ok(Json(out))
        }
        None => {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                "/admin/tokens/{id}",
                "admin_tokens_get",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            Err(ge)
        }
    }
}

pub async fn create_token(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateTokenPayload>,
) -> Result<(axum::http::StatusCode, Json<AdminTokenOut>), GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/tokens",
            "admin_tokens_create",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    if payload.id.is_some() {
        return Err(GatewayError::Config("不允许传入 id".into()));
    }
    let mut payload = payload;
    if let Some(name) = payload.name.as_deref() {
        payload.name = Some(validate_admin_token_name(name)?);
    }
    // 校验 allowed_models 存在性（若提供）
    if let Some(list) = payload.allowed_models.as_ref() && !list.is_empty() {
        use std::collections::HashSet;
        let cached = crate::server::model_cache::get_cached_models_all(&app_state)
            .await
            .map_err(GatewayError::Db)?;
        let set: HashSet<String> = cached.into_iter().map(|m| m.id).collect();
        for m in list {
            if !set.contains(m) {
                return Err(GatewayError::NotFound(format!(
                    "model '{}' not found in cache",
                    m
                )));
            }
        }
    }
    let t = app_state
        .token_store
        .create_token(CreateTokenPayload {
            id: None,
            token: None,
            ..payload
        })
        .await?;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/tokens",
        "admin_tokens_create",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        201,
        None,
    )
    .await;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(AdminTokenOut::from(t)),
    ))
}

#[derive(Debug, Deserialize)]
pub struct TogglePayload {
    pub enabled: bool,
}

pub async fn toggle_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<TogglePayload>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/tokens/{id}/toggle",
            "admin_tokens_toggle",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    let ok = app_state
        .token_store
        .set_enabled_by_id(&id, payload.enabled)
        .await?;
    if ok {
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/tokens/{id}/toggle",
            "admin_tokens_toggle",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            200,
            None,
        )
        .await;
        Ok(Json(serde_json::json!({"status":"ok"})))
    } else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/tokens/{id}/toggle",
            "admin_tokens_toggle",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        Err(ge)
    }
}

pub async fn delete_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            "/admin/tokens/{id}",
            "admin_tokens_delete",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    let deleted = app_state.token_store.delete_token_by_id(&id).await?;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            "/admin/tokens/{id}",
            "admin_tokens_delete",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            204,
            None,
        )
        .await;
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        let ge = GatewayError::NotFound("token not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            "/admin/tokens/{id}",
            "admin_tokens_delete",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(ge.to_string()),
        )
        .await;
        Err(ge)
    }
}

pub async fn update_token(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateTokenPayload>,
) -> Result<Json<AdminTokenOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            "/admin/tokens/{id}",
            "admin_tokens_update",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }
    if payload.id.is_some() {
        return Err(GatewayError::Config("不允许修改 id".into()));
    }
    let mut payload = payload;
    if let Some(name) = payload.name.as_deref() {
        payload.name = Some(validate_admin_token_name(name)?);
    }
    // 若更新了 allowed_models，需要校验
    if let Some(list) = payload.allowed_models.as_ref() {
        use std::collections::HashSet;
        let cached = crate::server::model_cache::get_cached_models_all(&app_state)
            .await
            .map_err(GatewayError::Db)?;
        let set: HashSet<String> = cached.into_iter().map(|m| m.id).collect();
        for m in list {
            if !set.contains(m) {
                return Err(GatewayError::NotFound(format!(
                    "model '{}' not found in cache",
                    m
                )));
            }
        }
    }
    match app_state.token_store.update_token_by_id(&id, payload).await? {
        Some(t) => {
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                "/admin/tokens/{id}",
                "admin_tokens_update",
                None,
                None,
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            Ok(Json(AdminTokenOut::from(t)))
        }
        None => {
            let ge = GatewayError::NotFound("token not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                "/admin/tokens/{id}",
                "admin_tokens_update",
                None,
                None,
                provided_token.as_deref(),
                code,
                Some(ge.to_string()),
            )
            .await;
            Err(ge)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{LoadBalancing, LoggingConfig, ServerConfig};
    use crate::config::BalanceStrategy;
    use crate::logging::DatabaseLogger;
    use crate::server::login::LoginManager;
    use crate::server::storage_traits::{AdminPublicKeyRecord, LoginStore, TuiSessionRecord};
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use chrono::{Duration, Utc};
    use tempfile::tempdir;

    fn test_settings(db_path: String) -> crate::config::Settings {
        crate::config::Settings {
            load_balancing: LoadBalancing {
                strategy: BalanceStrategy::FirstAvailable,
            },
            server: ServerConfig::default(),
            logging: LoggingConfig {
                database_path: db_path,
                ..Default::default()
            },
        }
    }

    struct Harness {
        _dir: tempfile::TempDir,
        state: Arc<AppState>,
        token: String,
    }

    async fn harness() -> Harness {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings = test_settings(db_path.to_str().unwrap().to_string());
        let logger = Arc::new(DatabaseLogger::new(&settings.logging.database_path).await.unwrap());

        let fingerprint = "test-fp".to_string();
        let now = Utc::now();
        logger
            .insert_admin_key(&AdminPublicKeyRecord {
                fingerprint: fingerprint.clone(),
                public_key: vec![0u8; ed25519_dalek::PUBLIC_KEY_LENGTH],
                comment: Some("test".into()),
                enabled: true,
                created_at: now,
                last_used_at: None,
            })
            .await
            .unwrap();

        let token = "test-admin-token".to_string();
        logger
            .create_tui_session(&TuiSessionRecord {
                session_id: token.clone(),
                fingerprint,
                issued_at: now,
                expires_at: now + Duration::hours(1),
                revoked: false,
                last_code_at: None,
            })
            .await
            .unwrap();

        let app_state = Arc::new(AppState {
            config: settings,
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger,
        });

        Harness {
            _dir: dir,
            state: app_state,
            token,
        }
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn admin_tokens_create_get_update_delete_works() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let (code, Json(created)) = create_token(
            State(h.state.clone()),
            headers.clone(),
            Json(CreateTokenPayload {
                id: None,
                name: Some("  my-token  ".into()),
                token: None,
                allowed_models: None,
                max_tokens: None,
                max_amount: Some(10.0),
                enabled: true,
                expires_at: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(code, axum::http::StatusCode::CREATED);
        assert!(created.id.starts_with("atk_"));
        assert_eq!(created.name, "my-token");
        assert_eq!(created.token.len(), 40);

        let Json(fetched) = get_token(
            Path(created.id.clone()),
            State(h.state.clone()),
            headers.clone(),
        )
        .await
        .unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, created.name);
        assert_eq!(fetched.token, created.token);

        let Json(updated) = update_token(
            Path(created.id.clone()),
            State(h.state.clone()),
            headers.clone(),
            Json(UpdateTokenPayload {
                id: None,
                name: Some("renamed".into()),
                allowed_models: None,
                max_tokens: None,
                max_amount: None,
                enabled: None,
                expires_at: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name, "renamed");

        let code = delete_token(Path(created.id.clone()), State(h.state.clone()), headers.clone())
            .await
            .unwrap();
        assert_eq!(code, axum::http::StatusCode::NO_CONTENT);

        let err = get_token(Path(created.id), State(h.state), headers).await.unwrap_err();
        assert!(matches!(err, GatewayError::NotFound(_)));
    }

    #[tokio::test]
    async fn admin_tokens_reject_client_supplied_id_and_empty_name() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let err = create_token(
            State(h.state.clone()),
            headers.clone(),
            Json(CreateTokenPayload {
                id: Some("client-id".into()),
                name: Some("name".into()),
                token: None,
                allowed_models: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GatewayError::Config(_)));

        let err = create_token(
            State(h.state),
            headers,
            Json(CreateTokenPayload {
                id: None,
                name: Some("   ".into()),
                token: None,
                allowed_models: None,
                max_tokens: None,
                max_amount: None,
                enabled: true,
                expires_at: None,
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, GatewayError::Config(_)));
    }
}
