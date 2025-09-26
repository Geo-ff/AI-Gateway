use rusqlite::{Connection, Result, OptionalExtension};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::providers::openai::Model;

#[derive(Debug, Clone)]
pub struct RequestLog {
    #[allow(dead_code)]
    pub id: Option<i64>,
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub status_code: u16,
    pub response_time_ms: i64,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct CachedModel {
    pub id: String,
    pub provider: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
    #[allow(dead_code)]
    pub cached_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct DatabaseLogger {
    connection: Arc<Mutex<Connection>>,
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
                log.timestamp.to_rfc3339(),
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

    pub async fn cache_models(&self, provider: &str, models: &[Model]) -> Result<()> {
        let conn = self.connection.lock().await;
        let now = Utc::now();

        // 清除该供应商的旧缓存
        conn.execute("DELETE FROM cached_models WHERE provider = ?1", [provider])?;

        // 插入新的模型数据
        for model in models {
            conn.execute(
                "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &model.id,
                    provider,
                    &model.object,
                    model.created,
                    &model.owned_by,
                    now.to_rfc3339(),
                ),
            )?;
        }

        tracing::info!("Cached {} models for provider: {}", models.len(), provider);
        Ok(())
    }

    pub async fn get_cached_models(&self, provider: Option<&str>) -> Result<Vec<CachedModel>> {
        let conn = self.connection.lock().await;

        if let Some(provider) = provider {
            let mut stmt = conn.prepare(
                "SELECT id, provider, object, created, owned_by, cached_at
                 FROM cached_models WHERE provider = ?1
                 ORDER BY id"
            )?;

            let model_iter = stmt.query_map([provider], |row| {
                Ok(CachedModel {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    object: row.get(2)?,
                    created: row.get(3)?,
                    owned_by: row.get(4)?,
                    cached_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })?;

            let mut models = Vec::new();
            for model in model_iter {
                models.push(model?);
            }

            Ok(models)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, provider, object, created, owned_by, cached_at
                 FROM cached_models
                 ORDER BY provider, id"
            )?;

            let model_iter = stmt.query_map([], |row| {
                Ok(CachedModel {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    object: row.get(2)?,
                    created: row.get(3)?,
                    owned_by: row.get(4)?,
                    cached_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })?;

            let mut models = Vec::new();
            for model in model_iter {
                models.push(model?);
            }

            Ok(models)
        }
    }

    pub async fn is_cache_fresh(&self, provider: &str, max_age_minutes: i64) -> Result<bool> {
        let conn = self.connection.lock().await;

        let mut stmt = conn.prepare(
            "SELECT cached_at FROM cached_models WHERE provider = ?1 LIMIT 1"
        )?;

        let cache_time: Option<String> = stmt.query_row([provider], |row| {
            Ok(row.get(0)?)
        }).optional()?;

        if let Some(cached_at_str) = cache_time {
            let cached_at = DateTime::parse_from_rfc3339(&cached_at_str)
                .unwrap()
                .with_timezone(&Utc);

            let age = Utc::now() - cached_at;
            Ok(age.num_minutes() < max_age_minutes)
        } else {
            Ok(false)
        }
    }

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
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                    .unwrap()
                    .with_timezone(&Utc),
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
