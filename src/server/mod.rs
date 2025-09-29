pub mod handlers;
pub mod login;
pub(crate) mod model_cache;
pub(crate) mod model_helpers;
pub(crate) mod model_parser;
pub(crate) mod model_redirect;
pub(crate) mod provider_dispatch;
pub(crate) mod request_logging;
pub(crate) mod storage_traits;
pub(crate) mod streaming;
pub(crate) mod util;

use crate::admin::{PgTokenStore, TokenStore};
use crate::config::Settings;
use crate::error::{GatewayError, Result as AppResult};
use crate::logging::DatabaseLogger;
use crate::logging::postgres_store::PgLogStore;
use crate::server::storage_traits::{
    AdminPublicKeyRecord, LoginStore, ModelCache, ProviderStore, RequestLogStore,
};
use axum::Router;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use chrono::Utc;
use ed25519_dalek::SigningKey;
use rand::Rng;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Settings,
    pub log_store: Arc<dyn RequestLogStore + Send + Sync>,
    pub model_cache: Arc<dyn ModelCache + Send + Sync>,
    pub providers: Arc<dyn ProviderStore + Send + Sync>,
    pub token_store: Arc<dyn TokenStore + Send + Sync>,
    pub admin_identity_token: String,
    pub login_manager: Arc<login::LoginManager>,
}

pub async fn create_app(config: Settings) -> AppResult<Router> {
    // Admin identity token generated per boot
    let admin_identity_token: String = {
        use rand::Rng;
        let rng = rand::rng();
        use rand::distr::Alphanumeric;
        rng.sample_iter(&Alphanumeric)
            .take(56)
            .map(char::from)
            .collect()
    };

    // Choose stores based on Postgres availability
    let (log_store_arc, model_cache_arc, provider_store_arc, token_store, login_store_arc): (
        Arc<dyn RequestLogStore + Send + Sync>,
        Arc<dyn ModelCache + Send + Sync>,
        Arc<dyn ProviderStore + Send + Sync>,
        Arc<dyn TokenStore + Send + Sync>,
        Arc<dyn LoginStore + Send + Sync>,
    ) = if let Some(pg_url) = &config.logging.pg_url {
        // Strict Postgres-only mode (no SQLite fallback)
        let pool_size = config.logging.pg_pool_size.unwrap_or(4);
        let pglog = PgLogStore::connect(pg_url, &config.logging.pg_schema, pool_size).await?;
        tracing::info!("Using PostgreSQL for logs and cache");
        let log_cache = Arc::new(pglog);
        let ts = PgTokenStore::connect(pg_url, config.logging.pg_schema.as_deref()).await?;
        (
            log_cache.clone(),
            log_cache.clone(),
            log_cache.clone(),
            Arc::new(ts),
            log_cache.clone(),
        )
    } else {
        let db_logger = Arc::new(DatabaseLogger::new(&config.logging.database_path).await?);
        (
            db_logger.clone(),
            db_logger.clone(),
            db_logger.clone(),
            db_logger.clone(),
            db_logger.clone(),
        )
    };

    if let Some((fingerprint, private_b64, path)) =
        ensure_initial_admin_key(login_store_arc.clone()).await?
    {
        tracing::warn!(
            "新管理员密钥已生成；指纹={}，私钥已写入 {}，请立即妥善备份并加载至 TUI 配置。",
            fingerprint,
            path.display()
        );
        tracing::warn!("一次性私钥（base64）：{}", private_b64);
    }

    tracing::info!(
        "Admin Identity Token (use as Bearer): {}",
        admin_identity_token
    );

    let app_state = AppState {
        config,
        log_store: log_store_arc,
        model_cache: model_cache_arc,
        providers: provider_store_arc,
        token_store,
        admin_identity_token,
        login_manager: Arc::new(login::LoginManager::new(login_store_arc.clone())),
    };

    let mut app = handlers::routes().with_state(Arc::new(app_state));

    // CORS（开发环境便于前端联调；生产应收敛来源并仅 HTTPS）
    use axum::http::{Method, header};
    use tower_http::cors::{AllowOrigin, CorsLayer};
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
        // 反射请求来源（便于 dev server 代理转发携带 Cookie）
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(true);
    app = app.layer(cors);

    Ok(app)
}

async fn ensure_initial_admin_key(
    login_store: Arc<dyn LoginStore + Send + Sync>,
) -> Result<Option<(String, String, PathBuf)>, GatewayError> {
    let existing = login_store
        .list_admin_keys()
        .await
        .map_err(GatewayError::Db)?;
    if !existing.is_empty() {
        return Ok(None);
    }

    let mut seed = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
    rand::rng().fill(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_bytes();
    let fingerprint = crate::server::login::LoginManager::fingerprint_for_public_key(&public_key);
    let record = AdminPublicKeyRecord {
        fingerprint: fingerprint.clone(),
        public_key: public_key.to_vec(),
        comment: Some("generated-on-boot".into()),
        enabled: true,
        created_at: Utc::now(),
        last_used_at: None,
    };
    login_store
        .insert_admin_key(&record)
        .await
        .map_err(GatewayError::Db)?;

    let private_b64 = B64_STANDARD.encode(signing_key.to_bytes());
    let path = admin_key_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        std::fs::write(&path, format!("{}\n", private_b64))?;
    } else {
        tracing::warn!("检测到已有管理员私钥文件，未覆盖：{}", path.display());
    }

    Ok(Some((fingerprint, private_b64, path)))
}

fn admin_key_file_path() -> PathBuf {
    PathBuf::from("data").join("admin_ed25519.key")
}
