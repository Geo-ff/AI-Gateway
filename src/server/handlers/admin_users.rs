use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;
use std::sync::Arc;

use super::auth::ensure_admin;
use crate::error::GatewayError;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};
use crate::users::{CreateUserPayload, UpdateUserPayload, User};
use chrono::Utc;

#[derive(Debug, Serialize)]
pub struct UserOut {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    pub email: String,
    pub phone_number: String,
    pub status: crate::users::UserStatus,
    pub role: crate::users::UserRole,
    pub created_at: String,
    pub updated_at: String,
}

impl From<User> for UserOut {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            first_name: u.first_name,
            last_name: u.last_name,
            username: u.username,
            email: u.email,
            phone_number: u.phone_number,
            status: u.status,
            role: u.role,
            created_at: crate::logging::time::to_beijing_string(&u.created_at),
            updated_at: crate::logging::time::to_beijing_string(&u.updated_at),
        }
    }
}

pub async fn list_users(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserOut>>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            "/admin/users",
            "admin_users_list",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let users = app_state
        .user_store
        .list_users()
        .await?
        .into_iter()
        .map(UserOut::from)
        .collect();
    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/admin/users",
        "admin_users_list",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        200,
        None,
    )
    .await;
    Ok(Json(users))
}

pub async fn get_user(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<UserOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "GET",
            &format!("/admin/users/{}", id),
            "admin_users_get",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    match app_state.user_store.get_user(&id).await? {
        Some(u) => Ok(Json(UserOut::from(u))),
        None => {
            let ge = GatewayError::NotFound("user not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "GET",
                &format!("/admin/users/{}", id),
                "admin_users_get",
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

pub async fn create_user(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateUserPayload>,
) -> Result<(axum::http::StatusCode, Json<UserOut>), GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "POST",
            "/admin/users",
            "admin_users_create",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let user = app_state.user_store.create_user(payload).await?;
    log_simple_request(
        &app_state,
        start_time,
        "POST",
        "/admin/users",
        "admin_users_create",
        None,
        None,
        token_for_log(provided_token.as_deref()),
        201,
        None,
    )
    .await;
    Ok((axum::http::StatusCode::CREATED, Json(UserOut::from(user))))
}

pub async fn update_user(
    Path(id): Path<String>,
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateUserPayload>,
) -> Result<Json<UserOut>, GatewayError> {
    let start_time = Utc::now();
    let provided_token = bearer_token(&headers);
    if let Err(e) = ensure_admin(&headers, &app_state).await {
        let code = e.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "PUT",
            &format!("/admin/users/{}", id),
            "admin_users_update",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    match app_state.user_store.update_user(&id, payload).await? {
        Some(u) => {
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                &format!("/admin/users/{}", id),
                "admin_users_update",
                None,
                None,
                token_for_log(provided_token.as_deref()),
                200,
                None,
            )
            .await;
            Ok(Json(UserOut::from(u)))
        }
        None => {
            let ge = GatewayError::NotFound("user not found".into());
            let code = ge.status_code().as_u16();
            log_simple_request(
                &app_state,
                start_time,
                "PUT",
                &format!("/admin/users/{}", id),
                "admin_users_update",
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

pub async fn delete_user(
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
            &format!("/admin/users/{}", id),
            "admin_users_delete",
            None,
            None,
            provided_token.as_deref(),
            code,
            Some(e.to_string()),
        )
        .await;
        return Err(e);
    }

    let deleted = app_state.user_store.delete_user(&id).await?;
    if deleted {
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/admin/users/{}", id),
            "admin_users_delete",
            None,
            None,
            token_for_log(provided_token.as_deref()),
            204,
            None,
        )
        .await;
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        let ge = GatewayError::NotFound("user not found".into());
        let code = ge.status_code().as_u16();
        log_simple_request(
            &app_state,
            start_time,
            "DELETE",
            &format!("/admin/users/{}", id),
            "admin_users_delete",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BalanceStrategy;
    use crate::config::settings::{LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::login::LoginManager;
    use crate::server::storage_traits::{AdminPublicKeyRecord, LoginStore, TuiSessionRecord};
    use crate::users::{UserRole, UserStatus};
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
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

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
    async fn admin_users_requires_admin_auth() {
        let h = harness().await;
        let res = list_users(State(h.state), HeaderMap::new()).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn admin_users_create_and_list_works() {
        let h = harness().await;
        let headers = auth_headers(&h.token);

        let (code, Json(created)) = create_user(
            State(h.state.clone()),
            headers.clone(),
            Json(CreateUserPayload {
                first_name: Some("Bob".into()),
                last_name: Some("Builder".into()),
                username: Some("bob".into()),
                email: "bob@example.com".into(),
                phone_number: Some("+1-555-1111".into()),
                status: UserStatus::Active,
                role: UserRole::Admin,
            }),
        )
        .await
        .unwrap();
        assert_eq!(code, axum::http::StatusCode::CREATED);
        assert_eq!(created.email, "bob@example.com");

        let Json(list) = list_users(State(h.state), headers).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, created.id);
    }
}
