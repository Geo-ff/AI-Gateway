use rusqlite::{Connection, Result};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::logging::time::{to_beijing_string, parse_beijing_string};
use crate::logging::types::RequestLog;

#[derive(Clone)]
pub struct DatabaseLogger {
    pub(super) connection: Arc<Mutex<Connection>>,
}

impl DatabaseLogger {
    pub async fn new(database_path: &str) -> Result<Self> {
        // 确保数据库文件的目录存在
        if let Some(parent) = std::path::Path::new(database_path).parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                        Some(format!("Failed to create directory: {}", e))
                    ));
                }
                tracing::info!("Created database directory: {}", parent.display());
            }
        }

        let conn = Connection::open(database_path)?;
        tracing::info!("Database initialized at: {}", database_path);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS request_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                request_type TEXT NOT NULL DEFAULT 'chat_once',
                model TEXT,
                provider TEXT,
                api_key TEXT,
                status_code INTEGER NOT NULL,
                response_time_ms INTEGER NOT NULL,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                total_tokens INTEGER,
                cached_tokens INTEGER,
                reasoning_tokens INTEGER,
                error_message TEXT,
                client_token TEXT,
                amount_spent REAL
            )",
            [],
        )?;

        // 迁移：补充旧表缺失的 request_type 列（若已存在则忽略错误）
        let _ = conn.execute(
            "ALTER TABLE request_logs ADD COLUMN request_type TEXT NOT NULL DEFAULT 'chat_once'",
            [],
        );
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN api_key TEXT", []);
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN error_message TEXT", []);
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN cached_tokens INTEGER", []);
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN reasoning_tokens INTEGER", []);

        conn.execute(
            "CREATE TABLE IF NOT EXISTS cached_models (
                id TEXT NOT NULL,
                provider TEXT NOT NULL,
                object TEXT NOT NULL,
                created INTEGER NOT NULL,
                owned_by TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (id, provider)
            )",
            [],
        )?;

        // Provider keys table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_keys (
                provider TEXT NOT NULL,
                key_value TEXT NOT NULL,
                enc INTEGER NOT NULL DEFAULT 0,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                PRIMARY KEY (provider, key_value)
            )",
            [],
        )?;

        // Providers table (dynamic provider metadata)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS providers (
                name TEXT PRIMARY KEY,
                api_type TEXT NOT NULL,
                base_url TEXT NOT NULL,
                models_endpoint TEXT
            )",
            [],
        )?;

        // Provider operations audit logs
        conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_ops_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                provider TEXT,
                details TEXT
            )",
            [],
        )?;

        // Admin tokens table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS admin_tokens (
                token TEXT PRIMARY KEY,
                allowed_models TEXT,
                max_tokens INTEGER,
                enabled INTEGER NOT NULL DEFAULT 1,
                expires_at TEXT,
                created_at TEXT NOT NULL,
                max_amount REAL,
                amount_spent REAL DEFAULT 0,
                prompt_tokens_spent INTEGER DEFAULT 0,
                completion_tokens_spent INTEGER DEFAULT 0,
                total_tokens_spent INTEGER DEFAULT 0
            )",
            [],
        )?;

        // Schema migrations for request_logs: client_token + amount_spent columns
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN client_token TEXT", []);
        let _ = conn.execute("ALTER TABLE request_logs ADD COLUMN amount_spent REAL", []);
        // Pricing table for models
        conn.execute(
            "CREATE TABLE IF NOT EXISTS model_prices (
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_price_per_million REAL NOT NULL,
                completion_price_per_million REAL NOT NULL,
                currency TEXT,
                PRIMARY KEY (provider, model)
            )",
            [],
        )?;
        // Migration: add max_amount to admin_tokens if missing
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN max_amount REAL",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN amount_spent REAL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN prompt_tokens_spent INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN completion_tokens_spent INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN total_tokens_spent INTEGER DEFAULT 0",
            [],
        );

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn sum_spent_amount_by_client_token(&self, token: &str) -> Result<f64> {
        // Sum cost = sum(prompt_tokens/1e6*prompt_price + completion_tokens/1e6*completion_price)
        let conn = self.connection.lock().await;
        // Using COALESCE to treat NULL as 0
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(
                COALESCE(prompt_tokens,0) * COALESCE(pp.prompt_price_per_million, 0) / 1000000.0 +
                COALESCE(completion_tokens,0) * COALESCE(pp.completion_price_per_million, 0) / 1000000.0
            ), 0.0)
             FROM request_logs rl
             JOIN model_prices pp ON rl.provider = pp.provider AND rl.model = pp.model
             WHERE rl.client_token = ?1"
        )?;
        let mut rows = stmt.query([token])?;
        if let Some(row) = rows.next()? {
            let sum: f64 = row.get(0).unwrap_or(0.0);
            Ok(sum)
        } else {
            Ok(0.0)
        }
    }

    pub async fn log_request(&self, log: RequestLog) -> Result<i64> {
        let conn = self.connection.lock().await;

        conn.execute(
            "INSERT INTO request_logs (
                timestamp, method, path, request_type, model, provider,
                api_key, status_code, response_time_ms, prompt_tokens,
                completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                client_token, amount_spent
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                to_beijing_string(&log.timestamp),
                &log.method,
                &log.path,
                &log.request_type,
                &log.model,
                &log.provider,
                &log.api_key,
                log.status_code,
                log.response_time_ms,
                log.prompt_tokens,
                log.completion_tokens,
                log.total_tokens,
                log.cached_tokens,
                log.reasoning_tokens,
                &log.error_message,
                &log.client_token,
                &log.amount_spent,
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    // 模型缓存相关方法已拆分至 database_cache.rs

    #[allow(dead_code)]
    pub async fn get_recent_logs(&self, limit: i32) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;

        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, path, request_type, model, provider,
                    api_key, status_code, response_time_ms, prompt_tokens,
                    completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                    client_token, amount_spent
             FROM request_logs
             ORDER BY timestamp DESC
             LIMIT ?1"
        )?;

        let log_iter = stmt.query_map([limit], |row| {
            Ok(RequestLog {
                id: Some(row.get(0)?),
                timestamp: parse_beijing_string(&row.get::<_, String>(1)?)
                    .unwrap(),
                method: row.get(2)?,
                path: row.get(3)?,
                request_type: row.get(4)?,
                model: row.get(5)?,
                provider: row.get(6)?,
                api_key: row.get(7)?,
                status_code: row.get(8)?,
                response_time_ms: row.get(9)?,
                prompt_tokens: row.get(10)?,
                completion_tokens: row.get(11)?,
                total_tokens: row.get(12)?,
                cached_tokens: row.get(13)?,
                reasoning_tokens: row.get(14)?,
                error_message: row.get(15)?,
                client_token: row.get(16)?,
                amount_spent: row.get(17)?,
            })
        })?;

        let mut logs = Vec::new();
        for log in log_iter {
            logs.push(log?);
        }

        Ok(logs)
    }

    pub async fn sum_total_tokens_by_client_token(&self, token: &str) -> Result<u64> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(total_tokens), 0) FROM request_logs WHERE client_token = ?1",
        )?;
        let mut rows = stmt.query([token])?;
        if let Some(row) = rows.next()? {
            let sum: Option<i64> = row.get(0)?;
            Ok(sum.unwrap_or(0) as u64)
        } else {
            Ok(0)
        }
    }

    pub async fn get_logs_by_client_token(&self, token: &str, limit: i32) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, path, request_type, model, provider,
                    api_key, status_code, response_time_ms, prompt_tokens,
                    completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                    client_token, amount_spent
             FROM request_logs WHERE client_token = ?1 ORDER BY id DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(rusqlite::params![token, limit], |row| {
            Ok(RequestLog {
                id: Some(row.get(0)?),
                timestamp: parse_beijing_string(&row.get::<_, String>(1)?)
                    .unwrap(),
                method: row.get(2)?,
                path: row.get(3)?,
                request_type: row.get(4)?,
                model: row.get(5)?,
                provider: row.get(6)?,
                api_key: row.get(7)?,
                status_code: row.get(8)?,
                response_time_ms: row.get(9)?,
                prompt_tokens: row.get(10)?,
                completion_tokens: row.get(11)?,
                total_tokens: row.get(12)?,
                cached_tokens: row.get(13)?,
                reasoning_tokens: row.get(14)?,
                error_message: row.get(15)?,
                client_token: row.get(16)?,
                amount_spent: row.get(17)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows { out.push(r?); }
        Ok(out)
    }
}
