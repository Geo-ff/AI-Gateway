use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Duration, Utc};
use tokio_postgres::{Client, NoTls, Row};

use crate::config::settings::{KeyLogStrategy, Provider, ProviderType};
use crate::error::GatewayError;
use crate::logging::time::{parse_datetime_string, to_beijing_string};
use crate::logging::types::ProviderOpLog;
use crate::logging::{CachedModel, ProviderKeyStatsAgg, RequestLog};
use crate::providers::openai::Model;
use crate::routing::{KeyRotationStrategy, ProviderKeyEntry};
use crate::server::storage_traits::{
    AdminPublicKeyRecord, BoxFuture, FavoriteKind, FavoritesStore, LoginCodeRecord, LoginStore,
    ModelCache, ProviderKeyEntryWithCreatedAt, ProviderStore, RequestLogStore, TuiSessionRecord,
    WebSessionRecord,
};

fn pg_err<E: std::fmt::Display>(e: E) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
        Some(format!("{}", e)),
    )
}

fn pg_row_i64(row: &Row, idx: usize) -> Option<i64> {
    row.try_get::<usize, i64>(idx)
        .ok()
        .or_else(|| row.try_get::<usize, i32>(idx).ok().map(|v| v as i64))
}

fn pg_row_i64_or(row: &Row, idx: usize, default: i64) -> i64 {
    pg_row_i64(row, idx).unwrap_or(default)
}

fn pg_row_f64_or(row: &Row, idx: usize, default: f64) -> f64 {
    row.try_get::<usize, f64>(idx)
        .ok()
        .or_else(|| {
            row.try_get::<usize, i64>(idx)
                .ok()
                .map(|v| v as f64)
                .or_else(|| row.try_get::<usize, i32>(idx).ok().map(|v| v as f64))
        })
        .unwrap_or(default)
}

fn pg_row_u16_or(row: &Row, idx: usize, default: u16) -> u16 {
    pg_row_i64(row, idx)
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(default)
}

fn pg_row_u32_or(row: &Row, idx: usize, default: u32) -> u32 {
    pg_row_i64(row, idx)
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(default)
}

fn pg_row_u32_opt(row: &Row, idx: usize) -> Option<u32> {
    if let Ok(v) = row.try_get::<usize, Option<i64>>(idx) {
        return v.and_then(|v| u32::try_from(v).ok());
    }
    row.try_get::<usize, Option<i32>>(idx)
        .ok()
        .flatten()
        .map(|v| v as u32)
}

fn pg_row_bool_or(row: &Row, idx: usize, default: bool) -> bool {
    row.try_get::<usize, bool>(idx)
        .ok()
        .or_else(|| row.try_get::<usize, Option<bool>>(idx).ok().flatten())
        .unwrap_or(default)
}

fn pg_row_string(row: &Row, idx: usize) -> String {
    row.try_get::<usize, String>(idx).unwrap_or_default()
}

fn pg_row_opt_string(row: &Row, idx: usize) -> Option<String> {
    row.try_get::<usize, Option<String>>(idx)
        .ok()
        .flatten()
        .or_else(|| row.try_get::<usize, String>(idx).ok())
}

fn pg_row_datetime_or_now(row: &Row, idx: usize) -> DateTime<Utc> {
    if let Ok(ts) = row.try_get::<usize, DateTime<Utc>>(idx) {
        return ts;
    }
    if let Ok(Some(raw)) = row.try_get::<usize, Option<String>>(idx) {
        return parse_datetime_string(&raw).unwrap_or_else(|_| Utc::now());
    }
    if let Ok(raw) = row.try_get::<usize, String>(idx) {
        return parse_datetime_string(&raw).unwrap_or_else(|_| Utc::now());
    }
    Utc::now()
}

fn pg_row_opt_datetime(row: &Row, idx: usize) -> Option<DateTime<Utc>> {
    if let Ok(v) = row.try_get::<usize, Option<DateTime<Utc>>>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<usize, Option<String>>(idx) {
        return v.and_then(|raw| parse_datetime_string(&raw).ok());
    }
    row.try_get::<usize, String>(idx)
        .ok()
        .and_then(|raw| parse_datetime_string(&raw).ok())
}

fn pg_row_bytes(row: &Row, idx: usize) -> Vec<u8> {
    row.try_get::<usize, Vec<u8>>(idx).unwrap_or_default()
}

pub struct PgPool {
    clients: Vec<Arc<Client>>,
    next: AtomicUsize,
}

impl PgPool {
    async fn connect_many(
        pg_url: &str,
        schema: &Option<String>,
        size: usize,
    ) -> Result<Self, GatewayError> {
        let mut clients = Vec::with_capacity(size.max(1));
        for _ in 0..size.max(1) {
            let (client, connection) = tokio_postgres::connect(pg_url, NoTls)
                .await
                .map_err(|e| GatewayError::Config(format!("Failed to connect postgres: {}", e)))?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    tracing::error!("postgres connection error: {}", e);
                }
            });
            if let Some(s) = schema {
                client
                    .execute(&format!("SET search_path TO {}", s), &[])
                    .await
                    .map_err(|e| {
                        GatewayError::Config(format!("Failed to set search_path: {}", e))
                    })?;
            }
            let client = Arc::new(client);
            // improve: jittered keepalive to avoid herd effects
            crate::db::postgres::spawn_keepalive(Arc::clone(&client), 240, 420);
            clients.push(client);
        }
        Ok(Self {
            clients,
            next: AtomicUsize::new(0),
        })
    }

    pub fn pick(&self) -> Arc<Client> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.clients.len().max(1);
        Arc::clone(&self.clients[idx])
    }
}

#[derive(Clone)]
pub struct PgLogStore {
    pub pool: Arc<PgPool>,
}

impl PgLogStore {
    pub async fn connect(
        pg_url: &str,
        schema: &Option<String>,
        pool_size: usize,
    ) -> Result<Self, GatewayError> {
        let pool = PgPool::connect_many(pg_url, schema, pool_size).await?;
        let store = Self {
            pool: Arc::new(pool),
        };
        // init tables
        let client = store.pool.pick();
        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS request_logs (
                id SERIAL PRIMARY KEY,
                timestamp TEXT NOT NULL,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                request_type TEXT NOT NULL,
                model TEXT,
                provider TEXT,
                api_key TEXT,
                status_code INTEGER NOT NULL,
                response_time_ms BIGINT NOT NULL,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                total_tokens INTEGER,
                cached_tokens INTEGER,
                reasoning_tokens INTEGER,
                error_message TEXT,
                client_token TEXT,
                amount_spent DOUBLE PRECISION
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init request_logs: {}", e)))?;
        // best-effort migration for existing deployments
        let _ = client
            .execute(
                "ALTER TABLE request_logs ADD COLUMN amount_spent DOUBLE PRECISION",
                &[],
            )
            .await;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS cached_models (
                id TEXT NOT NULL,
                provider TEXT NOT NULL,
                object TEXT NOT NULL,
                created BIGINT NOT NULL,
                owned_by TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (id, provider)
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init cached_models: {}", e)))?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS provider_ops_logs (
                id SERIAL PRIMARY KEY,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                provider TEXT,
                details TEXT
            )"#,
                &[],
            )
            .await
            .map_err(|e| {
                GatewayError::Config(format!("Failed to init provider_ops_logs: {}", e))
            })?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS model_prices (
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_price_per_million DOUBLE PRECISION NOT NULL,
                completion_price_per_million DOUBLE PRECISION NOT NULL,
                currency TEXT,
                PRIMARY KEY (provider, model)
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init model_prices: {}", e)))?;

        // Providers & provider_keys tables
        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS providers (
                name TEXT PRIMARY KEY,
                display_name TEXT,
                api_type TEXT NOT NULL,
                base_url TEXT NOT NULL,
                models_endpoint TEXT,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                key_rotation_strategy TEXT NOT NULL DEFAULT 'weighted_sequential'
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init providers: {}", e)))?;
        // best-effort migrations for existing deployments
        let _ = client
            .execute(
                "ALTER TABLE providers ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT TRUE",
                &[],
            )
            .await;
        let _ = client
            .execute("ALTER TABLE providers ADD COLUMN display_name TEXT", &[])
            .await;
        let _ = client
            .execute(
                "ALTER TABLE providers ADD COLUMN key_rotation_strategy TEXT NOT NULL DEFAULT 'weighted_sequential'",
                &[],
            )
            .await;
        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS provider_keys (
                provider TEXT NOT NULL,
                key_value TEXT NOT NULL,
                enc BOOLEAN NOT NULL DEFAULT FALSE,
                active BOOLEAN NOT NULL DEFAULT TRUE,
                weight INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                PRIMARY KEY (provider, key_value)
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init provider_keys: {}", e)))?;
        let _ = client
            .execute(
                "ALTER TABLE provider_keys ADD COLUMN weight INTEGER NOT NULL DEFAULT 1",
                &[],
            )
            .await;

        // Favorites table (used by admin UI)
        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS favorites (
                kind TEXT NOT NULL,
                target TEXT NOT NULL,
                favorite BOOLEAN NOT NULL DEFAULT TRUE,
                PRIMARY KEY (kind, target)
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init favorites: {}", e)))?;
        // Best-effort migrations for existing deployments
        let _ = client
            .execute(
                "ALTER TABLE favorites ADD COLUMN favorite BOOLEAN NOT NULL DEFAULT TRUE",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "CREATE INDEX IF NOT EXISTS favorites_kind_favorite_idx ON favorites(kind, favorite)",
                &[],
            )
            .await;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS model_redirects (
                provider TEXT NOT NULL,
                source_model TEXT NOT NULL,
                target_model TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (provider, source_model)
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init model_redirects: {}", e)))?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS admin_public_keys (
                fingerprint TEXT PRIMARY KEY,
                public_key BYTEA NOT NULL,
                comment TEXT,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TIMESTAMPTZ NOT NULL,
                last_used_at TIMESTAMPTZ
            )"#,
                &[],
            )
            .await
            .map_err(|e| {
                GatewayError::Config(format!("Failed to init admin_public_keys: {}", e))
            })?;

        client.execute(
            r#"CREATE TABLE IF NOT EXISTS tui_sessions (
                session_id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL REFERENCES admin_public_keys(fingerprint) ON DELETE CASCADE,
                issued_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                revoked BOOLEAN NOT NULL DEFAULT FALSE,
                last_code_at TIMESTAMPTZ
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init tui_sessions: {}", e)))?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS login_codes (
                code_hash TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES tui_sessions(session_id) ON DELETE CASCADE,
                fingerprint TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                max_uses INTEGER NOT NULL,
                uses INTEGER NOT NULL DEFAULT 0,
                disabled BOOLEAN NOT NULL DEFAULT FALSE,
                hint TEXT
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init login_codes: {}", e)))?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS web_sessions (
                session_id TEXT PRIMARY KEY,
                fingerprint TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                revoked BOOLEAN NOT NULL DEFAULT FALSE,
                issued_by_code TEXT
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init web_sessions: {}", e)))?;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                first_name TEXT NOT NULL,
                last_name TEXT NOT NULL,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL UNIQUE,
                phone_number TEXT NOT NULL,
                status TEXT NOT NULL,
                role TEXT NOT NULL,
                password_hash TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init users: {}", e)))?;

        // Best-effort migrations for existing deployments
        let _ = client
            .execute("ALTER TABLE users ADD COLUMN password_hash TEXT", &[])
            .await;
        // Ensure there is at most one superadmin.
        let _ = client
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS users_one_superadmin_uidx ON users (role) WHERE role='superadmin'",
                &[],
            )
            .await;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS refresh_tokens (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                revoked_at TIMESTAMPTZ,
                replaced_by_id TEXT,
                last_used_at TIMESTAMPTZ
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init refresh_tokens: {}", e)))?;
        let _ = client
            .execute(
                "CREATE INDEX IF NOT EXISTS refresh_tokens_user_id_idx ON refresh_tokens (user_id)",
                &[],
            )
            .await;

        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS password_reset_tokens (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                token_hash TEXT NOT NULL UNIQUE,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                used_at TIMESTAMPTZ
            )"#,
                &[],
            )
            .await
            .map_err(|e| {
                GatewayError::Config(format!("Failed to init password_reset_tokens: {}", e))
            })?;
        let _ = client
            .execute(
                "CREATE INDEX IF NOT EXISTS password_reset_tokens_user_id_idx ON password_reset_tokens (user_id)",
                &[],
            )
            .await;

        Ok(store)
    }
}

impl PgLogStore {
    fn row_to_request_log(r: Row) -> RequestLog {
        RequestLog {
            id: pg_row_i64(&r, 0),
            timestamp: pg_row_datetime_or_now(&r, 1),
            method: pg_row_string(&r, 2),
            path: pg_row_string(&r, 3),
            request_type: pg_row_string(&r, 4),
            model: pg_row_opt_string(&r, 5),
            provider: pg_row_opt_string(&r, 6),
            api_key: pg_row_opt_string(&r, 7),
            status_code: pg_row_u16_or(&r, 8, 0),
            response_time_ms: pg_row_i64_or(&r, 9, 0),
            prompt_tokens: pg_row_u32_opt(&r, 10),
            completion_tokens: pg_row_u32_opt(&r, 11),
            total_tokens: pg_row_u32_opt(&r, 12),
            cached_tokens: pg_row_u32_opt(&r, 13),
            reasoning_tokens: pg_row_u32_opt(&r, 14),
            error_message: pg_row_opt_string(&r, 15),
            client_token: pg_row_opt_string(&r, 16),
            amount_spent: r.try_get::<usize, Option<f64>>(17).ok().flatten(),
        }
    }
}

impl RequestLogStore for PgLogStore {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO request_logs (timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
                    &[&to_beijing_string(&log.timestamp), &log.method, &log.path, &log.request_type, &log.model, &log.provider, &log.api_key, &i32::from(log.status_code), &log.response_time_ms, &log.prompt_tokens.map(|v| v as i32), &log.completion_tokens.map(|v| v as i32), &log.total_tokens.map(|v| v as i32), &log.cached_tokens.map(|v| v as i32), &log.reasoning_tokens.map(|v| v as i32), &log.error_message, &log.client_token, &log.amount_spent],
                )
                .await
                .map_err(pg_err)?;
            Ok(res as i64)
        })
    }

    fn get_recent_logs<'a>(
        &'a self,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = client
                .query(
                    "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs ORDER BY id DESC LIMIT $1",
                    &[&lim],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows.into_iter().map(Self::row_to_request_log).collect())
        })
    }

    fn get_recent_logs_with_cursor<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = if let Some(cursor_id) = cursor {
                let cursor_i32 = cursor_id as i32;
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE id < $1 ORDER BY id DESC LIMIT $2",
                        &[&cursor_i32, &lim],
                    )
                    .await
                    .map_err(pg_err)?
            } else {
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs ORDER BY id DESC LIMIT $1",
                        &[&lim],
                    )
                    .await
                    .map_err(pg_err)?
            };
            Ok(rows.into_iter().map(Self::row_to_request_log).collect())
        })
    }

    fn get_request_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = if let Some(cursor_id) = cursor {
                let cursor_i32 = cursor_id as i32;
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE id < $1 ORDER BY id DESC LIMIT $2",
                        &[&cursor_i32, &lim],
                    )
                    .await
                    .map_err(pg_err)?
            } else {
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs ORDER BY id DESC LIMIT $1",
                        &[&lim],
                    )
                    .await
                    .map_err(pg_err)?
            };
            Ok(rows.into_iter().map(Self::row_to_request_log).collect())
        })
    }

    fn get_logs_by_method_path<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = if let Some(cursor_id) = cursor {
                let cursor_i32 = cursor_id as i32;
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE method = $1 AND path = $2 AND id < $3 ORDER BY id DESC LIMIT $4",
                        &[&method, &path, &cursor_i32, &lim],
                    )
                    .await
                    .map_err(pg_err)?
            } else {
                client
                    .query(
                        "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE method = $1 AND path = $2 ORDER BY id DESC LIMIT $3",
                        &[&method, &path, &lim],
                    )
                    .await
                    .map_err(pg_err)?
            };
            Ok(rows.into_iter().map(Self::row_to_request_log).collect())
        })
    }

    fn sum_total_tokens_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<u64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_one("SELECT COALESCE(SUM(total_tokens), 0) FROM request_logs WHERE client_token = $1", &[&token])
                .await
                .map_err(pg_err)?;
            let sum = pg_row_i64_or(&row, 0, 0);
            Ok(sum.max(0) as u64)
        })
    }

    fn get_logs_by_client_token<'a>(
        &'a self,
        token: &'a str,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = client
                .query(
                    "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE client_token = $1 ORDER BY id DESC LIMIT $2",
                    &[&token, &lim],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows.into_iter().map(Self::row_to_request_log).collect())
        })
    }

    fn count_requests_by_client_token<'a>(
        &'a self,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, i64)>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT client_token, COUNT(*) AS cnt FROM request_logs WHERE client_token IS NOT NULL GROUP BY client_token",
                    &[],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows
                .into_iter()
                .filter_map(|row| {
                    let token = pg_row_opt_string(&row, 0);
                    let count = pg_row_i64_or(&row, 1, 0);
                    token.map(|t| (t, count))
                })
                .collect())
        })
    }

    fn get_request_log_date_range<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<(DateTime<Utc>, DateTime<Utc>)>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT MIN(timestamp), MAX(timestamp) FROM request_logs WHERE method = $1 AND path = $2",
                    &[&method, &path],
            )
                .await
                .map_err(pg_err)?;
            if let Some(row) = row {
                let min = pg_row_opt_datetime(&row, 0);
                let max = pg_row_opt_datetime(&row, 1);
                Ok(min.zip(max))
            } else {
                Ok(None)
            }
        })
    }

    fn aggregate_provider_key_stats<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        provider: &'a str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyStatsAgg>>> {
        Box::pin(async move {
            let since_str = since.as_ref().map(to_beijing_string);
            // 由于时间戳按秒存储（无小数），这里将上界向后推 1 秒以避免“同秒”被排除
            let until_str = until
                .as_ref()
                .map(|dt| to_beijing_string(&(*dt + Duration::seconds(1))));

            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT api_key,
                            COUNT(*)::bigint as total_requests,
                            SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END)::bigint as success_count,
                            SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END)::bigint as failure_count
                     FROM request_logs
                     WHERE method = $1
                       AND path = $2
                       AND provider = $3
                       AND api_key IS NOT NULL
                       AND ($4::text IS NULL OR timestamp >= $4::text)
                       AND ($5::text IS NULL OR timestamp < $5::text)
                     GROUP BY api_key",
                    &[&method, &path, &provider, &since_str, &until_str],
                )
                .await
                .map_err(pg_err)?;

            let mut out = Vec::new();
            for row in rows {
                let api_key = pg_row_opt_string(&row, 0).unwrap_or_default();
                let total = pg_row_i64_or(&row, 1, 0).max(0) as u64;
                let success = pg_row_i64_or(&row, 2, 0).max(0) as u64;
                let failure = pg_row_i64_or(&row, 3, 0).max(0) as u64;
                if !api_key.is_empty() {
                    out.push(ProviderKeyStatsAgg {
                        api_key,
                        total_requests: total,
                        success_count: success,
                        failure_count: failure,
                    });
                }
            }
            Ok(out)
        })
    }

    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO provider_ops_logs (timestamp, operation, provider, details) VALUES ($1,$2,$3,$4)",
                    &[&to_beijing_string(&op.timestamp), &op.operation, &op.provider, &op.details],
                )
                .await
                .map_err(pg_err)?;
            Ok(res as i64)
        })
    }

    fn get_provider_ops_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderOpLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = if let Some(cursor_id) = cursor {
                let cursor_i32 = cursor_id as i32;
                client
                    .query(
                        "SELECT id, timestamp, operation, provider, details FROM provider_ops_logs WHERE id < $1 ORDER BY id DESC LIMIT $2",
                        &[&cursor_i32, &lim],
                    )
                    .await
                    .map_err(pg_err)?
            } else {
                client
                    .query(
                        "SELECT id, timestamp, operation, provider, details FROM provider_ops_logs ORDER BY id DESC LIMIT $1",
                        &[&lim],
                    )
                    .await
                    .map_err(pg_err)?
            };
            Ok(rows
                .into_iter()
                .map(|row| {
                    // `Row::get` panics on type mismatch; prefer `try_get` to avoid
                    // disconnecting the client if the DB column type differs (e.g. TIMESTAMPTZ).
                    let id = row
                        .try_get::<usize, i64>(0)
                        .ok()
                        .or_else(|| row.try_get::<usize, i32>(0).ok().map(|v| v as i64));
                    let timestamp = if let Ok(ts) = row.try_get::<usize, DateTime<Utc>>(1) {
                        ts
                    } else if let Ok(raw) = row.try_get::<usize, String>(1) {
                        parse_datetime_string(&raw).unwrap_or_else(|_| chrono::Utc::now())
                    } else {
                        chrono::Utc::now()
                    };
                    ProviderOpLog {
                        id,
                        timestamp,
                        operation: row.try_get(2).unwrap_or_default(),
                        provider: row.try_get(3).ok(),
                        details: row.try_get(4).ok(),
                    }
                })
                .collect())
        })
    }

    fn upsert_model_price<'a>(
        &'a self,
        provider: &'a str,
        model: &'a str,
        prompt_price_per_million: f64,
        completion_price_per_million: f64,
        currency: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            // 尝试 UPDATE，若未影响行则 INSERT（兼容不支持 ON CONFLICT 的库）
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE model_prices SET prompt_price_per_million=$3, completion_price_per_million=$4, currency=$5 WHERE provider=$1 AND model=$2",
                    &[&provider, &model, &prompt_price_per_million, &completion_price_per_million, &currency],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO model_prices (provider, model, prompt_price_per_million, completion_price_per_million, currency) VALUES ($1,$2,$3,$4,$5)",
                        &[&provider, &model, &prompt_price_per_million, &completion_price_per_million, &currency],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn get_model_price<'a>(
        &'a self,
        provider: &'a str,
        model: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = $1 AND model = $2",
                    &[&provider, &model],
                )
                .await
                .map_err(pg_err)?;
            Ok(row.map(|r| {
                (
                    pg_row_f64_or(&r, 0, 0.0),
                    pg_row_f64_or(&r, 1, 0.0),
                    pg_row_opt_string(&r, 2),
                )
            }))
        })
    }

    fn list_model_prices<'a>(
        &'a self,
        provider: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>> {
        Box::pin(async move {
            let mut out = Vec::new();
            if let Some(p) = provider {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = $1 ORDER BY model",
                        &[&p],
                )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push((
                        pg_row_string(&r, 0),
                        pg_row_string(&r, 1),
                        pg_row_f64_or(&r, 2, 0.0),
                        pg_row_f64_or(&r, 3, 0.0),
                        pg_row_opt_string(&r, 4),
                    ));
                }
            } else {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices ORDER BY provider, model",
                        &[],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push((
                        pg_row_string(&r, 0),
                        pg_row_string(&r, 1),
                        pg_row_f64_or(&r, 2, 0.0),
                        pg_row_f64_or(&r, 3, 0.0),
                        pg_row_opt_string(&r, 4),
                    ));
                }
            }
            Ok(out)
        })
    }

    fn sum_spent_amount_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<f64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_one(
                    "SELECT COALESCE(SUM(COALESCE(prompt_tokens,0) * COALESCE(pp.prompt_price_per_million,0) / 1000000.0 + COALESCE(completion_tokens,0) * COALESCE(pp.completion_price_per_million,0) / 1000000.0), 0.0)
                     FROM request_logs rl JOIN model_prices pp ON rl.provider = pp.provider AND rl.model = pp.model WHERE rl.client_token = $1",
                    &[&token],
                )
                .await
                .map_err(pg_err)?;
            Ok(pg_row_f64_or(&row, 0, 0.0))
        })
    }
}

impl FavoritesStore for PgLogStore {
    fn set_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
        favorite: bool,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            client
                .execute(
                    "INSERT INTO favorites (kind, target, favorite)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (kind, target) DO UPDATE SET favorite = EXCLUDED.favorite",
                    &[&kind.as_str(), &target, &favorite],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn is_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT favorite FROM favorites WHERE kind = $1 AND target = $2",
                    &[&kind.as_str(), &target],
                )
                .await
                .map_err(pg_err)?;
            Ok(row
                .and_then(|r| r.try_get::<usize, bool>(0).ok())
                .unwrap_or(false))
        })
    }

    fn list_favorites<'a>(
        &'a self,
        kind: FavoriteKind,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT target FROM favorites WHERE kind = $1 AND favorite = TRUE",
                    &[&kind.as_str()],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(pg_row_string(&r, 0));
            }
            Ok(out)
        })
    }
}

impl ModelCache for PgLogStore {
    fn cache_models<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = Utc::now();
            let client = self.pool.pick();
            client
                .execute(
                    "DELETE FROM cached_models WHERE provider = $1",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            for m in models {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at) VALUES ($1,$2,$3,$4,$5,$6)",
                        &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn get_cached_models<'a>(
        &'a self,
        provider: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
        Box::pin(async move {
            let mut out = Vec::new();
            if let Some(p) = provider {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT id, provider, object, created, owned_by, cached_at FROM cached_models WHERE provider = $1 ORDER BY id",
                        &[&p],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push(CachedModel {
                        id: pg_row_string(&r, 0),
                        provider: pg_row_string(&r, 1),
                        object: pg_row_string(&r, 2),
                        created: pg_row_i64_or(&r, 3, 0).max(0) as u64,
                        owned_by: pg_row_string(&r, 4),
                        cached_at: pg_row_datetime_or_now(&r, 5),
                    });
                }
            } else {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT id, provider, object, created, owned_by, cached_at FROM cached_models ORDER BY provider, id",
                        &[],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push(CachedModel {
                        id: pg_row_string(&r, 0),
                        provider: pg_row_string(&r, 1),
                        object: pg_row_string(&r, 2),
                        created: pg_row_i64_or(&r, 3, 0).max(0) as u64,
                        owned_by: pg_row_string(&r, 4),
                        cached_at: pg_row_datetime_or_now(&r, 5),
                    });
                }
            }
            Ok(out)
        })
    }

    fn cache_models_append<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = Utc::now();
            for m in models {
                // 尝试 UPDATE，若未影响行则 INSERT
                let client = self.pool.pick();
                let affected = client
                    .execute(
                        "UPDATE cached_models SET object=$3, created=$4, owned_by=$5, cached_at=$6 WHERE id=$1 AND provider=$2",
                        &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                    )
                    .await
                    .map_err(pg_err)?;
                if affected == 0 {
                    let client = self.pool.pick();
                    client
                        .execute(
                            "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at) VALUES ($1,$2,$3,$4,$5,$6)",
                            &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                        )
                        .await
                        .map_err(pg_err)?;
                }
            }
            Ok(())
        })
    }

    fn remove_cached_models<'a>(
        &'a self,
        provider: &'a str,
        ids: &'a [String],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            if ids.is_empty() {
                client
                    .execute(
                        "DELETE FROM cached_models WHERE provider = $1",
                        &[&provider],
                    )
                    .await
                    .map_err(pg_err)?;
            } else {
                for id in ids {
                    client
                        .execute(
                            "DELETE FROM cached_models WHERE provider = $1 AND id = $2",
                            &[&provider, id],
                        )
                        .await
                        .map_err(pg_err)?;
                }
            }
            Ok(())
        })
    }
}

impl ProviderStore for PgLogStore {
    fn insert_provider<'a>(
        &'a self,
        provider: &'a Provider,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO providers (name, display_name, api_type, base_url, models_endpoint) VALUES ($1,$2,$3,$4,$5)",
                    &[&provider.name, &provider.display_name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                )
                .await
                .map_err(pg_err)?;
            Ok(res > 0)
        })
    }

    fn upsert_provider<'a>(
        &'a self,
        provider: &'a Provider,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE providers SET display_name=$2, api_type=$3, base_url=$4, models_endpoint=$5 WHERE name=$1",
                    &[&provider.name, &provider.display_name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO providers (name, display_name, api_type, base_url, models_endpoint) VALUES ($1,$2,$3,$4,$5)",
                        &[&provider.name, &provider.display_name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt("SELECT 1 FROM providers WHERE name = $1 LIMIT 1", &[&name])
                .await
                .map_err(pg_err)?;
            Ok(row.is_some())
        })
    }

    fn get_provider<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt("SELECT name, display_name, api_type, base_url, models_endpoint, enabled FROM providers WHERE name = $1", &[&name])
                .await
                .map_err(pg_err)?;
            Ok(row.map(|r| Provider {
                name: pg_row_string(&r, 0),
                display_name: pg_row_opt_string(&r, 1),
                api_type: provider_type_from_str(&pg_row_string(&r, 2)),
                base_url: pg_row_string(&r, 3),
                api_keys: Vec::new(),
                models_endpoint: pg_row_opt_string(&r, 4),
                enabled: pg_row_bool_or(&r, 5, true),
            }))
        })
    }

    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT name, display_name, api_type, base_url, models_endpoint, enabled FROM providers ORDER BY name",
                    &[],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(Provider {
                    name: pg_row_string(&r, 0),
                    display_name: pg_row_opt_string(&r, 1),
                    api_type: provider_type_from_str(&pg_row_string(&r, 2)),
                    base_url: pg_row_string(&r, 3),
                    api_keys: Vec::new(),
                    models_endpoint: pg_row_opt_string(&r, 4),
                    enabled: pg_row_bool_or(&r, 5, true),
                });
            }
            Ok(out)
        })
    }

    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            // cascade-like cleanup
            let client = self.pool.pick();
            client
                .execute("DELETE FROM provider_keys WHERE provider = $1", &[&name])
                .await
                .map_err(pg_err)?;
            let client = self.pool.pick();
            client
                .execute("DELETE FROM cached_models WHERE provider = $1", &[&name])
                .await
                .map_err(pg_err)?;
            let client = self.pool.pick();
            client
                .execute("DELETE FROM model_redirects WHERE provider = $1", &[&name])
                .await
                .map_err(pg_err)?;
            let client = self.pool.pick();
            let res = client
                .execute("DELETE FROM providers WHERE name = $1", &[&name])
                .await
                .map_err(pg_err)?;
            Ok(res > 0)
        })
    }

    fn set_provider_enabled<'a>(
        &'a self,
        provider: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let affected = client
                .execute(
                    "UPDATE providers SET enabled = $2 WHERE name = $1",
                    &[&provider, &enabled],
                )
                .await
                .map_err(pg_err)?;
            Ok(affected > 0)
        })
    }

    fn get_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<KeyRotationStrategy>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT key_rotation_strategy FROM providers WHERE name = $1 LIMIT 1",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            let value = row.as_ref().map(|r| pg_row_string(r, 0));
            Ok(KeyRotationStrategy::from_db_value(value.as_deref()))
        })
    }

    fn set_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
        strategy: KeyRotationStrategy,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let affected = client
                .execute(
                    "UPDATE providers SET key_rotation_strategy = $2 WHERE name = $1",
                    &[&provider, &strategy.as_db_value()],
                )
                .await
                .map_err(pg_err)?;
            Ok(affected > 0)
        })
    }

    fn get_provider_keys<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query("SELECT key_value, enc FROM provider_keys WHERE provider = $1 AND active = TRUE ORDER BY created_at", &[&provider])
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                let value = pg_row_string(&r, 0);
                let enc = pg_row_bool_or(&r, 1, false);
                let decrypted =
                    crate::crypto::unprotect(strategy, provider, &value, enc).unwrap_or_default();
                if !decrypted.is_empty() {
                    out.push(decrypted);
                }
            }
            Ok(out)
        })
    }

    fn add_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = to_beijing_string(&Utc::now());
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE provider_keys SET enc=$3, active=TRUE, created_at=$4 WHERE provider=$1 AND key_value=$2",
                    &[&provider, &stored, &enc, &now],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO provider_keys (provider, key_value, enc, active, weight, created_at) VALUES ($1,$2,$3,TRUE,1,$4)",
                        &[&provider, &stored, &enc, &now],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn remove_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let mut affected = client
                .execute(
                    "DELETE FROM provider_keys WHERE provider = $1 AND key_value = $2",
                    &[&provider, &stored],
                )
                .await
                .map_err(pg_err)?;
            if enc {
                let client = self.pool.pick();
                affected += client
                    .execute(
                        "DELETE FROM provider_keys WHERE provider = $1 AND key_value = $2",
                        &[&provider, &key],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(affected > 0)
        })
    }

    fn list_provider_keys_raw<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntry>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT key_value, enc, active, weight FROM provider_keys WHERE provider = $1 ORDER BY created_at",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                let value = pg_row_string(&r, 0);
                let enc = pg_row_bool_or(&r, 1, false);
                let active = pg_row_bool_or(&r, 2, true);
                let weight = pg_row_u32_or(&r, 3, 1);
                let decrypted =
                    crate::crypto::unprotect(strategy, provider, &value, enc).unwrap_or_default();
                if !decrypted.is_empty() {
                    out.push(ProviderKeyEntry {
                        value: decrypted,
                        active,
                        weight: if weight >= 1 { weight } else { 1 },
                    });
                }
            }
            Ok(out)
        })
    }

    fn list_provider_keys_raw_with_created_at<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntryWithCreatedAt>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT key_value, enc, active, weight, created_at FROM provider_keys WHERE provider = $1 ORDER BY created_at",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                let value = pg_row_string(&r, 0);
                let enc = pg_row_bool_or(&r, 1, false);
                let active = pg_row_bool_or(&r, 2, true);
                let weight = pg_row_u32_or(&r, 3, 1);
                let created_at_raw = pg_row_string(&r, 4);
                let created_at = parse_datetime_string(&created_at_raw).map_err(pg_err)?;

                let decrypted =
                    crate::crypto::unprotect(strategy, provider, &value, enc).unwrap_or_default();
                if !decrypted.is_empty() {
                    out.push(ProviderKeyEntryWithCreatedAt {
                        value: decrypted,
                        active,
                        weight: if weight >= 1 { weight } else { 1 },
                        created_at,
                    });
                }
            }
            Ok(out)
        })
    }

    fn set_provider_key_weight<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        weight: u32,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            // 将 u32 转换为 i32，若超出范围则返回业务错误
            let weight_i32: i32 = weight.try_into().map_err(|_| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                    Some(format!("weight {} exceeds i32::MAX", weight)),
                )
            })?;
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let mut affected = client
                .execute(
                    "UPDATE provider_keys SET weight = $3 WHERE provider = $1 AND key_value = $2",
                    &[&provider, &stored, &weight_i32],
                )
                .await
                .map_err(pg_err)?;
            if enc {
                let client = self.pool.pick();
                affected += client
                    .execute(
                        "UPDATE provider_keys SET weight = $3 WHERE provider = $1 AND key_value = $2",
                        &[&provider, &key, &weight_i32],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(affected > 0)
        })
    }

    fn set_provider_key_active<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        active: bool,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let mut affected = client
                .execute(
                    "UPDATE provider_keys SET active = $3 WHERE provider = $1 AND key_value = $2",
                    &[&provider, &stored, &active],
                )
                .await
                .map_err(pg_err)?;
            if enc {
                let client = self.pool.pick();
                affected += client
                    .execute(
                        "UPDATE provider_keys SET active = $3 WHERE provider = $1 AND key_value = $2",
                        &[&provider, &key, &active],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(affected > 0)
        })
    }

    fn list_model_redirects<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String)>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT source_model, target_model FROM model_redirects WHERE provider = $1 ORDER BY source_model",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push((pg_row_string(&r, 0), pg_row_string(&r, 1)));
            }
            Ok(out)
        })
    }

    fn replace_model_redirects<'a>(
        &'a self,
        provider: &'a str,
        redirects: &'a [(String, String)],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now_s = to_beijing_string(&now);
            let client = self.pool.pick();
            client
                .execute(
                    "DELETE FROM model_redirects WHERE provider = $1",
                    &[&provider],
                )
                .await
                .map_err(pg_err)?;
            for (source, target) in redirects {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO model_redirects (provider, source_model, target_model, created_at, updated_at) VALUES ($1,$2,$3,$4,$5)",
                        &[&provider, source, target, &now_s, &now_s],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn delete_model_redirect<'a>(
        &'a self,
        provider: &'a str,
        source_model: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let affected = client
                .execute(
                    "DELETE FROM model_redirects WHERE provider = $1 AND source_model = $2",
                    &[&provider, &source_model],
                )
                .await
                .map_err(pg_err)?;
            Ok(affected > 0)
        })
    }
}

impl LoginStore for PgLogStore {
    fn insert_admin_key<'a>(
        &'a self,
        key: &'a AdminPublicKeyRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let comment = key.comment.as_deref();
            // 先尝试 UPDATE，兼容不支持 ON CONFLICT 的老版本 Postgres
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE admin_public_keys
                     SET public_key=$2, comment=$3, enabled=$4, created_at=$5, last_used_at=$6
                     WHERE fingerprint=$1",
                    &[
                        &key.fingerprint,
                        &key.public_key,
                        &comment,
                        &key.enabled,
                        &key.created_at,
                        &key.last_used_at,
                    ],
                )
                .await
                .map_err(pg_err)?;

            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO admin_public_keys (fingerprint, public_key, comment, enabled, created_at, last_used_at)
                         VALUES ($1, $2, $3, $4, $5, $6)",
                        &[&key.fingerprint, &key.public_key, &comment, &key.enabled, &key.created_at, &key.last_used_at],
                    )
                    .await
                    .map_err(pg_err)?;
            }

            Ok(())
        })
    }

    fn get_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<AdminPublicKeyRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT fingerprint, public_key, comment, enabled, created_at, last_used_at FROM admin_public_keys WHERE fingerprint = $1",
                    &[&fingerprint],
            )
                .await
                .map_err(pg_err)?;
            let rec = row.map(|r| AdminPublicKeyRecord {
                fingerprint: pg_row_string(&r, 0),
                public_key: pg_row_bytes(&r, 1),
                comment: pg_row_opt_string(&r, 2),
                enabled: pg_row_bool_or(&r, 3, true),
                created_at: pg_row_datetime_or_now(&r, 4),
                last_used_at: pg_row_opt_datetime(&r, 5),
            });
            Ok(rec)
        })
    }

    fn touch_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
        when: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            client
                .execute(
                    "UPDATE admin_public_keys SET last_used_at = $2 WHERE fingerprint = $1",
                    &[&fingerprint, &when],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn list_admin_keys<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<AdminPublicKeyRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query(
                    "SELECT fingerprint, public_key, comment, enabled, created_at, last_used_at FROM admin_public_keys",
                    &[],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                out.push(AdminPublicKeyRecord {
                    fingerprint: pg_row_string(&r, 0),
                    public_key: pg_row_bytes(&r, 1),
                    comment: pg_row_opt_string(&r, 2),
                    enabled: pg_row_bool_or(&r, 3, true),
                    created_at: pg_row_datetime_or_now(&r, 4),
                    last_used_at: pg_row_opt_datetime(&r, 5),
                });
            }
            Ok(out)
        })
    }

    fn delete_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .execute(
                    "DELETE FROM admin_public_keys WHERE fingerprint = $1",
                    &[&fingerprint],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows > 0)
        })
    }

    fn create_tui_session<'a>(
        &'a self,
        session: &'a TuiSessionRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            client
                .execute(
                    "INSERT INTO tui_sessions (session_id, fingerprint, issued_at, expires_at, revoked, last_code_at)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    &[&session.session_id, &session.fingerprint, &session.issued_at, &session.expires_at, &session.revoked, &session.last_code_at],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn get_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<TuiSessionRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions WHERE session_id = $1",
                    &[&session_id],
            )
                .await
                .map_err(pg_err)?;
            let rec = row.map(|r| TuiSessionRecord {
                session_id: pg_row_string(&r, 0),
                fingerprint: pg_row_string(&r, 1),
                issued_at: pg_row_datetime_or_now(&r, 2),
                expires_at: pg_row_datetime_or_now(&r, 3),
                revoked: pg_row_bool_or(&r, 4, false),
                last_code_at: pg_row_opt_datetime(&r, 5),
            });
            Ok(rec)
        })
    }

    fn list_tui_sessions<'a>(
        &'a self,
        fingerprint: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<TuiSessionRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = match fingerprint {
                Some(fp) => {
                    client
                        .query(
                            "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions WHERE fingerprint = $1 ORDER BY issued_at DESC",
                            &[&fp],
                        )
                        .await
                        .map_err(pg_err)?
                }
                None => {
                    client
                        .query(
                            "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions ORDER BY issued_at DESC",
                            &[],
                        )
                        .await
                        .map_err(pg_err)?
                }
            };
            let mut out = Vec::with_capacity(rows.len());
            for r in rows {
                out.push(TuiSessionRecord {
                    session_id: pg_row_string(&r, 0),
                    fingerprint: pg_row_string(&r, 1),
                    issued_at: pg_row_datetime_or_now(&r, 2),
                    expires_at: pg_row_datetime_or_now(&r, 3),
                    revoked: pg_row_bool_or(&r, 4, false),
                    last_code_at: pg_row_opt_datetime(&r, 5),
                });
            }
            Ok(out)
        })
    }

    fn update_tui_session_last_code<'a>(
        &'a self,
        session_id: &'a str,
        when: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            client
                .execute(
                    "UPDATE tui_sessions SET last_code_at = $2 WHERE session_id = $1",
                    &[&session_id, &when],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn revoke_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .execute(
                    "UPDATE tui_sessions SET revoked = TRUE WHERE session_id = $1",
                    &[&session_id],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows > 0)
        })
    }

    fn disable_codes_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            client
                .execute(
                    "UPDATE login_codes SET disabled = TRUE WHERE session_id = $1 AND disabled = FALSE",
                    &[&session_id],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn insert_login_code<'a>(
        &'a self,
        code: &'a LoginCodeRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let hint = code.hint.as_deref();
            client
                .execute(
                    "INSERT INTO login_codes (code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                    &[&code.code_hash, &code.session_id, &code.fingerprint, &code.created_at, &code.expires_at, &(code.max_uses as i32), &(code.uses as i32), &code.disabled, &hint],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn redeem_login_code<'a>(
        &'a self,
        code_hash: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "UPDATE login_codes
                     SET uses = uses + 1,
                         disabled = (uses + 1 >= max_uses) OR (expires_at <= $2)
                     WHERE code_hash = $1
                       AND disabled = FALSE
                       AND expires_at > $2
                       AND uses < max_uses
                     RETURNING code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint",
                    &[&code_hash, &now],
                )
                .await
                .map_err(pg_err)?;

            if let Some(r) = row {
                let record = LoginCodeRecord {
                    code_hash: pg_row_string(&r, 0),
                    session_id: pg_row_string(&r, 1),
                    fingerprint: pg_row_string(&r, 2),
                    created_at: pg_row_datetime_or_now(&r, 3),
                    expires_at: pg_row_datetime_or_now(&r, 4),
                    max_uses: pg_row_u32_or(&r, 5, 0),
                    uses: pg_row_u32_or(&r, 6, 0),
                    disabled: pg_row_bool_or(&r, 7, false),
                    hint: pg_row_opt_string(&r, 8),
                };
                return Ok(Some(record));
            }

            client
                .execute(
                    "UPDATE login_codes SET disabled = TRUE WHERE code_hash = $1 AND (disabled = FALSE) AND (expires_at <= $2 OR uses >= max_uses)",
                    &[&code_hash, &now],
                )
                .await
                .map_err(pg_err)?;
            Ok(None)
        })
    }

    fn get_latest_login_code_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint
                     FROM login_codes WHERE session_id = $1 ORDER BY created_at DESC LIMIT 1",
                    &[&session_id],
                )
                .await
                .map_err(pg_err)?;

            let rec = row.map(|r| LoginCodeRecord {
                code_hash: pg_row_string(&r, 0),
                session_id: pg_row_string(&r, 1),
                fingerprint: pg_row_string(&r, 2),
                created_at: pg_row_datetime_or_now(&r, 3),
                expires_at: pg_row_datetime_or_now(&r, 4),
                max_uses: pg_row_u32_or(&r, 5, 0),
                uses: pg_row_u32_or(&r, 6, 0),
                disabled: pg_row_bool_or(&r, 7, false),
                hint: pg_row_opt_string(&r, 8),
            });
            Ok(rec)
        })
    }

    fn insert_web_session<'a>(
        &'a self,
        session: &'a WebSessionRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let fingerprint = session.fingerprint.as_deref();
            let issued_by = session.issued_by_code.as_deref();
            client
                .execute(
                    "INSERT INTO web_sessions (session_id, fingerprint, created_at, expires_at, revoked, issued_by_code)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    &[&session.session_id, &fingerprint, &session.created_at, &session.expires_at, &session.revoked, &issued_by],
                )
                .await
                .map_err(pg_err)?;
            Ok(())
        })
    }

    fn get_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<WebSessionRecord>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT session_id, fingerprint, created_at, expires_at, revoked, issued_by_code FROM web_sessions WHERE session_id = $1",
                    &[&session_id],
            )
                .await
                .map_err(pg_err)?;
            let rec = row.map(|r| WebSessionRecord {
                session_id: pg_row_string(&r, 0),
                fingerprint: pg_row_opt_string(&r, 1),
                created_at: pg_row_datetime_or_now(&r, 2),
                expires_at: pg_row_datetime_or_now(&r, 3),
                revoked: pg_row_bool_or(&r, 4, false),
                issued_by_code: pg_row_opt_string(&r, 5),
            });
            Ok(rec)
        })
    }

    fn revoke_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .execute(
                    "UPDATE web_sessions SET revoked = TRUE WHERE session_id = $1",
                    &[&session_id],
                )
                .await
                .map_err(pg_err)?;
            Ok(rows > 0)
        })
    }
}

fn provider_type_from_str(s: &str) -> ProviderType {
    match s.to_ascii_lowercase().as_str() {
        "openai" => ProviderType::OpenAI,
        "anthropic" => ProviderType::Anthropic,
        "zhipu" => ProviderType::Zhipu,
        _ => ProviderType::OpenAI,
    }
}

fn provider_type_to_str(t: &ProviderType) -> &'static str {
    match t {
        ProviderType::OpenAI => "openai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Zhipu => "zhipu",
    }
}
