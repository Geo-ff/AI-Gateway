use rusqlite::{Connection, Result, OptionalExtension};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::providers::openai::Model;
use crate::logging::time::{to_beijing_string, parse_beijing_string};
use crate::logging::types::{CachedModel, RequestLog};

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
                model TEXT,
                provider TEXT,
                status_code INTEGER NOT NULL,
                response_time_ms INTEGER NOT NULL,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                total_tokens INTEGER
            )",
            [],
        )?;

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

        Ok(Self {
            connection: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn log_request(&self, log: RequestLog) -> Result<i64> {
        let conn = self.connection.lock().await;

        conn.execute(
            "INSERT INTO request_logs (
                timestamp, method, path, model, provider,
                status_code, response_time_ms, prompt_tokens,
                completion_tokens, total_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                to_beijing_string(&log.timestamp),
                &log.method,
                &log.path,
                &log.model,
                &log.provider,
                log.status_code,
                log.response_time_ms,
                log.prompt_tokens,
                log.completion_tokens,
                log.total_tokens,
            ),
        )?;

        Ok(conn.last_insert_rowid())
    }

    // 模型缓存相关方法已拆分至 database_cache.rs

    #[allow(dead_code)]
    pub async fn get_recent_logs(&self, limit: i32) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;

        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, path, model, provider,
                    status_code, response_time_ms, prompt_tokens,
                    completion_tokens, total_tokens
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
                model: row.get(4)?,
                provider: row.get(5)?,
                status_code: row.get(6)?,
                response_time_ms: row.get(7)?,
                prompt_tokens: row.get(8)?,
                completion_tokens: row.get(9)?,
                total_tokens: row.get(10)?,
            })
        })?;

        let mut logs = Vec::new();
        for log in log_iter {
            logs.push(log?);
        }

        Ok(logs)
    }
}
