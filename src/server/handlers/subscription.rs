use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;

use super::auth::require_user;
use crate::balance::BalanceTransactionKind;
use crate::error::GatewayError;
use crate::logging::RequestLog;
use crate::logging::types::REQ_TYPE_RECHARGE;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;
use crate::server::util::{bearer_token, token_for_log};
use crate::subscription::SubscriptionPlan;

async fn log_recharge_request(
    app_state: &AppState,
    start_time: DateTime<Utc>,
    user_id: Option<&str>,
    status_code: u16,
    amount_spent: Option<f64>,
    error_message: Option<String>,
) {
    let end_time = Utc::now();
    let response_time_ms = (end_time - start_time).num_milliseconds();
    let log = RequestLog {
        id: None,
        timestamp: start_time,
        method: "POST".to_string(),
        path: "/subscription/purchase".to_string(),
        request_type: REQ_TYPE_RECHARGE.to_string(),
        requested_model: None,
        effective_model: None,
        model: None,
        provider: None,
        api_key: None,
        client_token: None,
        user_id: user_id.map(|s| s.to_string()),
        amount_spent,
        status_code,
        response_time_ms,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        cached_tokens: None,
        reasoning_tokens: None,
        error_message,
    };

    if let Err(e) = app_state.log_store.log_request(log).await {
        tracing::error!("Failed to log recharge request: {}", e);
    }
}

pub async fn list_plans(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let provided = bearer_token(&headers);

    let published = app_state.subscription_store.get_published_plans().await?;
    let plans: Vec<SubscriptionPlan> = published.plans;

    log_simple_request(
        &app_state,
        start_time,
        "GET",
        "/subscription/plans",
        "subscription_plans",
        None,
        None,
        token_for_log(provided.as_deref()),
        200,
        None,
    )
    .await;

    Ok(Json(serde_json::json!({
        "updated_at": crate::logging::time::to_iso8601_utc_string(&published.updated_at),
        "plans": plans,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseRequest {
    pub plan_id: String,
}

pub async fn purchase_plan(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PurchaseRequest>,
) -> Result<Json<serde_json::Value>, GatewayError> {
    let start_time = Utc::now();
    let claims = match require_user(&headers) {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_recharge_request(
                &app_state,
                start_time,
                None,
                code,
                None,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    let published = app_state.subscription_store.get_published_plans().await?;
    let Some(plan) = published
        .plans
        .iter()
        .find(|p| p.plan_id == payload.plan_id)
        .cloned()
    else {
        let ge = GatewayError::NotFound("plan not found".into());
        let code = ge.status_code().as_u16();
        log_recharge_request(
            &app_state,
            start_time,
            Some(&claims.sub),
            code,
            None,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    };

    if plan.credits <= 0.0 {
        let ge = GatewayError::Config("invalid plan credits".into());
        let code = ge.status_code().as_u16();
        log_recharge_request(
            &app_state,
            start_time,
            Some(&claims.sub),
            code,
            plan.price_cny,
            Some(ge.to_string()),
        )
        .await;
        return Err(ge);
    }

    let new_balance = match app_state
        .user_store
        .add_balance(&claims.sub, plan.credits)
        .await?
    {
        Some(v) => v,
        None => {
            let ge = GatewayError::Unauthorized("invalid credentials".into());
            let code = ge.status_code().as_u16();
            log_recharge_request(
                &app_state,
                start_time,
                Some(&claims.sub),
                code,
                plan.price_cny,
                Some(ge.to_string()),
            )
            .await;
            return Err(ge);
        }
    };

    let meta = serde_json::json!({
        "plan_id": plan.plan_id,
        "plan_name": plan.name,
        "plan_price_cny": plan.price_cny,
        "plan_credits": plan.credits,
    })
    .to_string();
    let tx = match app_state
        .balance_store
        .create_transaction(
            &claims.sub,
            BalanceTransactionKind::Topup,
            plan.credits,
            Some(meta),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            let code = e.status_code().as_u16();
            log_recharge_request(
                &app_state,
                start_time,
                Some(&claims.sub),
                code,
                plan.price_cny,
                Some(e.to_string()),
            )
            .await;
            return Err(e);
        }
    };

    log_recharge_request(
        &app_state,
        start_time,
        Some(&claims.sub),
        200,
        plan.price_cny,
        None,
    )
    .await;

    Ok(Json(serde_json::json!({
        "balance": new_balance,
        "transaction_id": tx.id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{CreateTokenPayload, TokenStore};
    use crate::config::settings::{BalanceStrategy, LoadBalancing, LoggingConfig, ServerConfig};
    use crate::logging::DatabaseLogger;
    use crate::server::AppState;
    use crate::server::login::LoginManager;
    use crate::subscription::SubscriptionPlan;
    use crate::users::{CreateUserPayload, UserRole, UserStatus, UserStore};
    use axum::body::Body;
    use axum::extract::Query;
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use axum::http::{Request, StatusCode};
    use chrono::{Duration, Utc};
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt;

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

    #[tokio::test]
    async fn purchase_increases_balance_and_creates_transaction() {
        unsafe {
            std::env::set_var("GW_JWT_SECRET", "testsecret");
        }

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings = test_settings(db_path.to_str().unwrap().to_string());
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        let user = logger
            .create_user(CreateUserPayload {
                first_name: Some("U".into()),
                last_name: Some("1".into()),
                username: None,
                email: "u1@example.com".into(),
                phone_number: None,
                password: None,
                status: UserStatus::Active,
                role: UserRole::Admin,
                is_anonymous: false,
            })
            .await
            .unwrap();

        let created_token = logger
            .create_token(CreateTokenPayload {
                id: None,
                user_id: Some(user.id.clone()),
                name: Some("t1".into()),
                token: None,
                allowed_models: None,
                model_blacklist: None,
                max_tokens: None,
                max_amount: None,
                enabled: false,
                expires_at: None,
                remark: None,
                organization_id: None,
                ip_whitelist: None,
                ip_blacklist: None,
            })
            .await
            .unwrap();

        let plan = SubscriptionPlan {
            plan_id: "p1".into(),
            name: "Plan 1".into(),
            price_cny: Some(9.9),
            credits: 12.5,
            tagline: None,
            features: None,
        };
        let _ = app_state
            .subscription_store
            .put_draft_plans(vec![plan.clone()], Some("test".into()))
            .await
            .unwrap();
        let _ = app_state
            .subscription_store
            .publish_draft(Some("test".into()))
            .await
            .unwrap();

        let now = Utc::now();
        let claims = super::super::auth::AccessTokenClaims {
            sub: user.id.clone(),
            email: user.email.clone(),
            role: "user".into(),
            permissions: Vec::new(),
            jti: None,
            exp: (now + Duration::minutes(30)).timestamp(),
            iat: Some(now.timestamp()),
        };
        let access_token = super::super::auth::issue_access_token(&claims).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", access_token)).unwrap(),
        );

        let Json(out) = purchase_plan(
            State(app_state.clone()),
            headers,
            Json(PurchaseRequest {
                plan_id: "p1".into(),
            }),
        )
        .await
        .unwrap();

        assert!((out["balance"].as_f64().unwrap() - 12.5).abs() < 1e-9);
        let tx_id = out["transaction_id"].as_str().unwrap();
        assert!(!tx_id.is_empty());

        let updated = logger.get_user(&user.id).await.unwrap().unwrap();
        assert!((updated.balance - 12.5).abs() < 1e-9);

        let txs = app_state
            .balance_store
            .list_transactions(&user.id, 10, 0)
            .await
            .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].kind.as_str(), "topup");
        assert!((txs[0].amount - 12.5).abs() < 1e-9);
        let meta = txs[0].meta.as_deref().unwrap_or("");
        let v: serde_json::Value = serde_json::from_str(meta).unwrap();
        assert_eq!(v["plan_id"].as_str().unwrap(), "p1");
        assert!((v["plan_credits"].as_f64().unwrap() - 12.5).abs() < 1e-9);
        assert!((v["plan_price_cny"].as_f64().unwrap() - 9.9).abs() < 1e-9);

        // 购买/充值后不自动启用任何密钥，必须手动启用
        let fetched = logger
            .get_token(&created_token.token)
            .await
            .unwrap()
            .unwrap();
        assert!(!fetched.enabled);

        // 充值请求写入日志：amount_spent=credits 且带 user_id（用于管理端展示使用用户）
        let logs = logger.get_recent_logs_with_cursor(20, None).await.unwrap();
        let recharge_log = logs
            .iter()
            .find(|l| l.request_type == "recharge" && l.path == "/subscription/purchase")
            .expect("missing recharge request log");
        assert_eq!(recharge_log.user_id.as_deref(), Some(user.id.as_str()));
        assert_eq!(recharge_log.amount_spent, Some(9.9));

        // 管理端请求日志返回 username（通过 user_id 回填）
        let admin_claims = super::super::auth::AccessTokenClaims {
            sub: "sa".into(),
            email: "sa@example.com".into(),
            role: "superadmin".into(),
            permissions: Vec::new(),
            jti: None,
            exp: (now + Duration::minutes(30)).timestamp(),
            iat: Some(now.timestamp()),
        };
        let admin_access_token = super::super::auth::issue_access_token(&admin_claims).unwrap();
        let mut admin_headers = HeaderMap::new();
        admin_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", admin_access_token)).unwrap(),
        );
        let Json(admin_out) = crate::server::handlers::admin_logs::list_request_logs(
            State(app_state.clone()),
            admin_headers,
            Query(crate::server::handlers::admin_logs::LogsQuery {
                limit: Some(200),
                cursor: None,
                request_type: Some("recharge".into()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        let admin_recharge = admin_out
            .data
            .iter()
            .find(|l| l.request_type == "recharge" && l.path == "/subscription/purchase")
            .expect("missing admin recharge log entry");
        assert_eq!(admin_recharge.amount_spent, Some(9.9));
        assert_eq!(
            admin_recharge.username.as_deref(),
            Some(user.username.as_str())
        );
    }

    #[tokio::test]
    async fn subscription_routes_exist_for_root_and_api_prefix() {
        unsafe {
            std::env::set_var("GW_JWT_SECRET", "testsecret");
        }

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings = test_settings(db_path.to_str().unwrap().to_string());
        let logger = Arc::new(
            DatabaseLogger::new(&settings.logging.database_path)
                .await
                .unwrap(),
        );

        let app_state = Arc::new(AppState {
            config: settings,
            load_balancer_state: Arc::new(crate::routing::LoadBalancerState::default()),
            log_store: logger.clone(),
            model_cache: logger.clone(),
            providers: logger.clone(),
            token_store: logger.clone(),
            favorites_store: logger.clone(),
            login_manager: Arc::new(LoginManager::new(logger.clone())),
            user_store: logger.clone(),
            refresh_token_store: logger.clone(),
            password_reset_token_store: logger.clone(),
            balance_store: logger.clone(),
            subscription_store: logger.clone(),
        });

        let routes = crate::server::handlers::routes();
        let app = axum::Router::new()
            .merge(routes.clone())
            .nest("/api", routes)
            .with_state(app_state);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/subscription/plans")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/subscription/plans")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // `/me/logs/requests` 路由必须存在：未登录时应返回 401 而不是 404
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/me/logs/requests")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/me/logs/requests")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}
