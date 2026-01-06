use crate::logging::time::{
    BEIJING_OFFSET, DATETIME_FORMAT, parse_beijing_string, to_beijing_string,
};
use crate::logging::types::RequestLog;
use crate::server::storage_traits::{
    AdminPublicKeyRecord, LoginCodeRecord, TuiSessionRecord, WebSessionRecord,
};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct DatabaseLogger {
    pub(super) connection: Arc<Mutex<Connection>>,
}

impl crate::server::storage_traits::LoginStore for DatabaseLogger {
    fn insert_admin_key<'a>(
        &'a self,
        key: &'a AdminPublicKeyRecord,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let created = encode_ts(&key.created_at);
            let last_used_val = key.last_used_at.as_ref().map(encode_ts);
            let last_used = last_used_val.as_deref();
            let comment = key.comment.as_deref();
            conn.execute(
                "INSERT OR REPLACE INTO admin_public_keys (fingerprint, public_key, comment, enabled, created_at, last_used_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    &key.fingerprint,
                    &key.public_key,
                    comment,
                    if key.enabled { 1 } else { 0 },
                    &created,
                    last_used,
                ],
            )?;
            Ok(())
        })
    }

    fn get_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Option<AdminPublicKeyRecord>>>
    {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT fingerprint, public_key, comment, enabled, created_at, last_used_at FROM admin_public_keys WHERE fingerprint = ?1",
            )?;
            let record = stmt
                .query_row([fingerprint], |row| {
                    let created_raw: String = row.get(4)?;
                    let last_used_raw: Option<String> = row.get(5)?;
                    let created_at = decode_ts(&created_raw)?;
                    let last_used_at = match last_used_raw {
                        Some(v) => Some(decode_ts(&v)?),
                        None => None,
                    };
                    Ok(AdminPublicKeyRecord {
                        fingerprint: row.get(0)?,
                        public_key: row.get(1)?,
                        comment: row.get::<_, Option<String>>(2)?,
                        enabled: row.get::<_, i64>(3)? != 0,
                        created_at,
                        last_used_at,
                    })
                })
                .optional()?;
            Ok(record)
        })
    }

    fn touch_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
        when: DateTime<Utc>,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let when_s = encode_ts(&when);
            conn.execute(
                "UPDATE admin_public_keys SET last_used_at = ?2 WHERE fingerprint = ?1",
                rusqlite::params![fingerprint, &when_s],
            )?;
            Ok(())
        })
    }

    fn list_admin_keys<'a>(
        &'a self,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Vec<AdminPublicKeyRecord>>>
    {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT fingerprint, public_key, comment, enabled, created_at, last_used_at FROM admin_public_keys",
            )?;
            let rows = stmt.query_map([], |row| {
                let created_raw: String = row.get(4)?;
                let last_used_raw: Option<String> = row.get(5)?;
                let created_at = decode_ts(&created_raw)?;
                let last_used_at = match last_used_raw {
                    Some(v) => Some(decode_ts(&v)?),
                    None => None,
                };
                Ok(AdminPublicKeyRecord {
                    fingerprint: row.get(0)?,
                    public_key: row.get(1)?,
                    comment: row.get::<_, Option<String>>(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    created_at,
                    last_used_at,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    fn delete_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let rows = conn.execute(
                "DELETE FROM admin_public_keys WHERE fingerprint = ?1",
                rusqlite::params![fingerprint],
            )?;
            Ok(rows > 0)
        })
    }

    fn create_tui_session<'a>(
        &'a self,
        session: &'a TuiSessionRecord,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let issued = encode_ts(&session.issued_at);
            let expires = encode_ts(&session.expires_at);
            let last_code_val = session.last_code_at.as_ref().map(encode_ts);
            let last_code = last_code_val.as_deref();
            conn.execute(
                "INSERT INTO tui_sessions (session_id, fingerprint, issued_at, expires_at, revoked, last_code_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    &session.session_id,
                    &session.fingerprint,
                    &issued,
                    &expires,
                    if session.revoked { 1 } else { 0 },
                    last_code,
                ],
            )?;
            Ok(())
        })
    }

    fn get_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Option<TuiSessionRecord>>>
    {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions WHERE session_id = ?1",
            )?;
            let rec = stmt
                .query_row([session_id], |row| {
                    let issued_raw: String = row.get(2)?;
                    let expires_raw: String = row.get(3)?;
                    let last_code_raw: Option<String> = row.get(5)?;
                    Ok(TuiSessionRecord {
                        session_id: row.get(0)?,
                        fingerprint: row.get(1)?,
                        issued_at: decode_ts(&issued_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        revoked: row.get::<_, i64>(4)? != 0,
                        last_code_at: match last_code_raw {
                            Some(v) => Some(decode_ts(&v)?),
                            None => None,
                        },
                    })
                })
                .optional()?;
            Ok(rec)
        })
    }

    fn update_tui_session_last_code<'a>(
        &'a self,
        session_id: &'a str,
        when: DateTime<Utc>,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let when_s = encode_ts(&when);
            conn.execute(
                "UPDATE tui_sessions SET last_code_at = ?2 WHERE session_id = ?1",
                rusqlite::params![session_id, &when_s],
            )?;
            Ok(())
        })
    }

    fn revoke_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let affected = conn.execute(
                "UPDATE tui_sessions SET revoked = 1 WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;
            Ok(affected > 0)
        })
    }

    fn list_tui_sessions<'a>(
        &'a self,
        fingerprint: Option<&'a str>,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Vec<TuiSessionRecord>>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut out = Vec::new();
            if let Some(fp) = fingerprint {
                let mut stmt = conn.prepare(
                    "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions WHERE fingerprint = ?1 ORDER BY issued_at DESC",
                )?;
                let mut rows = stmt.query([fp])?;
                while let Some(row) = rows.next()? {
                    let issued_raw: String = row.get(2)?;
                    let expires_raw: String = row.get(3)?;
                    let last_raw: Option<String> = row.get(5)?;
                    out.push(TuiSessionRecord {
                        session_id: row.get(0)?,
                        fingerprint: row.get(1)?,
                        issued_at: decode_ts(&issued_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        revoked: row.get::<_, i64>(4)? != 0,
                        last_code_at: match last_raw {
                            Some(s) => Some(decode_ts(&s)?),
                            None => None,
                        },
                    });
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT session_id, fingerprint, issued_at, expires_at, revoked, last_code_at FROM tui_sessions ORDER BY issued_at DESC",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let issued_raw: String = row.get(2)?;
                    let expires_raw: String = row.get(3)?;
                    let last_raw: Option<String> = row.get(5)?;
                    out.push(TuiSessionRecord {
                        session_id: row.get(0)?,
                        fingerprint: row.get(1)?,
                        issued_at: decode_ts(&issued_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        revoked: row.get::<_, i64>(4)? != 0,
                        last_code_at: match last_raw {
                            Some(s) => Some(decode_ts(&s)?),
                            None => None,
                        },
                    });
                }
            }
            Ok(out)
        })
    }

    fn disable_codes_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            conn.execute(
                "UPDATE login_codes SET disabled = 1 WHERE session_id = ?1 AND disabled = 0",
                rusqlite::params![session_id],
            )?;
            Ok(())
        })
    }

    fn insert_login_code<'a>(
        &'a self,
        code: &'a LoginCodeRecord,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let created = encode_ts(&code.created_at);
            let expires = encode_ts(&code.expires_at);
            conn.execute(
                "INSERT INTO login_codes (code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    &code.code_hash,
                    &code.session_id,
                    &code.fingerprint,
                    &created,
                    &expires,
                    code.max_uses as i64,
                    code.uses as i64,
                    if code.disabled { 1 } else { 0 },
                    code.hint.as_deref(),
                ],
            )?;
            Ok(())
        })
    }

    fn redeem_login_code<'a>(
        &'a self,
        code_hash: &'a str,
        now: DateTime<Utc>,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>>
    {
        Box::pin(async move {
            let mut conn = self.connection.lock().await;
            let tx = conn.transaction()?;
            let record_opt = {
                let mut stmt = tx.prepare(
                    "SELECT code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint FROM login_codes WHERE code_hash = ?1",
                )?;
                stmt.query_row([code_hash], |row| {
                    let created_raw: String = row.get(3)?;
                    let expires_raw: String = row.get(4)?;
                    Ok(LoginCodeRecord {
                        code_hash: row.get(0)?,
                        session_id: row.get(1)?,
                        fingerprint: row.get(2)?,
                        created_at: decode_ts(&created_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        max_uses: row.get::<_, i64>(5)? as u32,
                        uses: row.get::<_, i64>(6)? as u32,
                        disabled: row.get::<_, i64>(7)? != 0,
                        hint: row.get::<_, Option<String>>(8)?,
                    })
                })
                .optional()?
            };

            let mut record = match record_opt {
                Some(r) => r,
                None => {
                    tx.commit()?;
                    return Ok(None);
                }
            };

            let mut should_disable = record.disabled;
            if record.disabled || now > record.expires_at || record.uses >= record.max_uses {
                should_disable = true;
            } else {
                record.uses += 1;
                if record.uses >= record.max_uses {
                    should_disable = true;
                }
                if now > record.expires_at {
                    should_disable = true;
                }
            }

            if record.disabled || now > record.expires_at || record.uses > record.max_uses {
                tx.execute(
                    "UPDATE login_codes SET disabled = 1 WHERE code_hash = ?1",
                    rusqlite::params![code_hash],
                )?;
                tx.commit()?;
                return Ok(None);
            }

            tx.execute(
                "UPDATE login_codes SET uses = ?2, disabled = ?3 WHERE code_hash = ?1",
                rusqlite::params![
                    code_hash,
                    record.uses as i64,
                    if should_disable { 1 } else { 0 },
                ],
            )?;

            record.disabled = should_disable;
            tx.commit()?;
            Ok(Some(record))
        })
    }

    fn get_latest_login_code_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>>
    {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT code_hash, session_id, fingerprint, created_at, expires_at, max_uses, uses, disabled, hint
                 FROM login_codes WHERE session_id = ?1 ORDER BY created_at DESC LIMIT 1",
            )?;
            let rec = stmt
                .query_row([session_id], |row| {
                    let created_raw: String = row.get(3)?;
                    let expires_raw: String = row.get(4)?;
                    Ok(LoginCodeRecord {
                        code_hash: row.get(0)?,
                        session_id: row.get(1)?,
                        fingerprint: row.get(2)?,
                        created_at: decode_ts(&created_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        max_uses: row.get::<_, i64>(5)? as u32,
                        uses: row.get::<_, i64>(6)? as u32,
                        disabled: row.get::<_, i64>(7)? != 0,
                        hint: row.get::<_, Option<String>>(8)?,
                    })
                })
                .optional()?;
            Ok(rec)
        })
    }

    fn insert_web_session<'a>(
        &'a self,
        session: &'a WebSessionRecord,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let created = encode_ts(&session.created_at);
            let expires = encode_ts(&session.expires_at);
            conn.execute(
                "INSERT INTO web_sessions (session_id, fingerprint, created_at, expires_at, revoked, issued_by_code) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    &session.session_id,
                    session.fingerprint.as_deref(),
                    &created,
                    &expires,
                    if session.revoked { 1 } else { 0 },
                    session.issued_by_code.as_deref(),
                ],
            )?;
            Ok(())
        })
    }

    fn get_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<Option<WebSessionRecord>>>
    {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let mut stmt = conn.prepare(
                "SELECT session_id, fingerprint, created_at, expires_at, revoked, issued_by_code FROM web_sessions WHERE session_id = ?1",
            )?;
            let rec = stmt
                .query_row([session_id], |row| {
                    let created_raw: String = row.get(2)?;
                    let expires_raw: String = row.get(3)?;
                    Ok(WebSessionRecord {
                        session_id: row.get(0)?,
                        fingerprint: row.get::<_, Option<String>>(1)?,
                        created_at: decode_ts(&created_raw)?,
                        expires_at: decode_ts(&expires_raw)?,
                        revoked: row.get::<_, i64>(4)? != 0,
                        issued_by_code: row.get::<_, Option<String>>(5)?,
                    })
                })
                .optional()?;
            Ok(rec)
        })
    }

    fn revoke_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> crate::server::storage_traits::BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let conn = self.connection.lock().await;
            let affected = conn.execute(
                "UPDATE web_sessions SET revoked = 1 WHERE session_id = ?1",
                rusqlite::params![session_id],
            )?;
            Ok(affected > 0)
        })
    }
}

fn encode_ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn decode_ts(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

impl DatabaseLogger {
    #[allow(clippy::collapsible_if)]
    pub async fn new(database_path: &str) -> Result<Self> {
        // 确保数据库文件的目录存在
        if let Some(parent) = std::path::Path::new(database_path).parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                        Some(format!("Failed to create directory: {}", e)),
                    ));
                }
                tracing::info!("Created database directory: {}", parent.display());
            }
        }

        let conn = Connection::open(database_path)?;
        conn.execute("PRAGMA foreign_keys = ON", [])?;
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
        let _ = conn.execute(
            "ALTER TABLE request_logs ADD COLUMN cached_tokens INTEGER",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE request_logs ADD COLUMN reasoning_tokens INTEGER",
            [],
        );

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
                id TEXT,
                name TEXT,
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
                total_tokens_spent INTEGER DEFAULT 0,
                remark TEXT,
                organization_id TEXT,
                ip_whitelist TEXT,
                ip_blacklist TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                first_name TEXT NOT NULL,
                last_name TEXT NOT NULL,
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL UNIQUE,
                phone_number TEXT NOT NULL,
                status TEXT NOT NULL,
                role TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
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
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN max_amount REAL", []);
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
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN id TEXT", []);
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN name TEXT", []);
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN remark TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE admin_tokens ADD COLUMN organization_id TEXT",
            [],
        );
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN ip_whitelist TEXT", []);
        let _ = conn.execute("ALTER TABLE admin_tokens ADD COLUMN ip_blacklist TEXT", []);
        let _ = conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS admin_tokens_id_uidx ON admin_tokens(id)",
            [],
        );

        conn.execute(
            "CREATE TABLE IF NOT EXISTS admin_public_keys (
                fingerprint TEXT PRIMARY KEY,
                public_key BLOB NOT NULL,
                comment TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                last_used_at TEXT
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS tui_sessions (
                session_id TEXT PRIMARY KEY,
                fingerprint TEXT NOT NULL,
                issued_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0,
                last_code_at TEXT,
                FOREIGN KEY(fingerprint) REFERENCES admin_public_keys(fingerprint) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS login_codes (
                code_hash TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                max_uses INTEGER NOT NULL,
                uses INTEGER NOT NULL DEFAULT 0,
                disabled INTEGER NOT NULL DEFAULT 0,
                hint TEXT,
                FOREIGN KEY(session_id) REFERENCES tui_sessions(session_id) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS web_sessions (
                session_id TEXT PRIMARY KEY,
                fingerprint TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                revoked INTEGER NOT NULL DEFAULT 0,
                issued_by_code TEXT
            )",
            [],
        )?;

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
        self.get_recent_logs_with_cursor(limit, None).await
    }

    pub async fn get_recent_logs_with_cursor(
        &self,
        limit: i32,
        cursor: Option<i64>,
    ) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;
        let mut stmt = if cursor.is_some() {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 WHERE id < ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )?
        } else {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 ORDER BY id DESC
                 LIMIT ?1",
            )?
        };

        let rows = if let Some(cursor_id) = cursor {
            stmt.query_map(rusqlite::params![cursor_id, limit], map_request_log_row)?
        } else {
            stmt.query_map([limit], map_request_log_row)?
        };

        let mut logs = Vec::new();
        for log in rows {
            logs.push(log?);
        }

        Ok(logs)
    }

    #[allow(dead_code)]
    pub async fn get_request_logs(
        &self,
        limit: i32,
        cursor: Option<i64>,
    ) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;

        let mut stmt = if cursor.is_some() {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 WHERE id < ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )?
        } else {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 ORDER BY id DESC
                 LIMIT ?1",
            )?
        };

        let rows = if let Some(cursor_id) = cursor {
            stmt.query_map(rusqlite::params![cursor_id, limit], map_request_log_row)?
        } else {
            stmt.query_map([limit], map_request_log_row)?
        };

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub async fn get_logs_by_method_path(
        &self,
        method: &str,
        path: &str,
        limit: i32,
        cursor: Option<i64>,
    ) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;
        let mut stmt = if cursor.is_some() {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 WHERE method = ?1 AND path = ?2 AND id < ?3
                 ORDER BY id DESC
                 LIMIT ?4",
            )?
        } else {
            conn.prepare(
                "SELECT id, timestamp, method, path, request_type, model, provider,
                        api_key, status_code, response_time_ms, prompt_tokens,
                        completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                        client_token, amount_spent
                 FROM request_logs
                 WHERE method = ?1 AND path = ?2
                 ORDER BY id DESC
                 LIMIT ?3",
            )?
        };

        let rows = if let Some(cursor_id) = cursor {
            stmt.query_map(
                rusqlite::params![method, path, cursor_id, limit],
                map_request_log_row,
            )?
        } else {
            stmt.query_map(rusqlite::params![method, path, limit], map_request_log_row)?
        };

        let mut logs = Vec::new();
        for r in rows {
            logs.push(r?);
        }
        Ok(logs)
    }

    #[allow(dead_code)]
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

    pub async fn get_logs_by_client_token(
        &self,
        token: &str,
        limit: i32,
    ) -> Result<Vec<RequestLog>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, method, path, request_type, model, provider,
                    api_key, status_code, response_time_ms, prompt_tokens,
                    completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message,
                    client_token, amount_spent
             FROM request_logs WHERE client_token = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![token, limit], |row| {
            Ok(RequestLog {
                id: Some(row.get(0)?),
                timestamp: parse_beijing_string(&row.get::<_, String>(1)?).unwrap(),
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
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub async fn count_requests_by_client_token(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT client_token, COUNT(*) as cnt
             FROM request_logs
             WHERE client_token IS NOT NULL
             GROUP BY client_token",
        )?;
        let rows = stmt.query_map([], |row| {
            let token: Option<String> = row.get(0)?;
            let count: i64 = row.get(1)?;
            match token {
                Some(t) => Ok(Some((t, count))),
                None => Ok(None),
            }
        })?;

        let mut result = Vec::new();
        for row in rows {
            if let Some(entry) = row? {
                result.push(entry);
            }
        }
        Ok(result)
    }

    pub async fn request_log_date_range(
        &self,
        method: &str,
        path: &str,
    ) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT MIN(timestamp), MAX(timestamp) FROM request_logs WHERE method = ?1 AND path = ?2",
        )?;
        let mut rows = stmt.query((method, path))?;
        if let Some(row) = rows.next()? {
            let min_ts: Option<String> = row.get(0)?;
            let max_ts: Option<String> = row.get(1)?;
            match (min_ts, max_ts) {
                (Some(min_ts), Some(max_ts)) => {
                    let min = parse_ts(&min_ts)?;
                    let max = parse_ts(&max_ts)?;
                    Ok(Some((min, max)))
                }
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }
}

fn map_request_log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLog> {
    let ts: String = row.get(1)?;
    Ok(RequestLog {
        id: Some(row.get(0)?),
        timestamp: parse_beijing_string(&ts).unwrap_or_else(|_| chrono::Utc::now()),
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
}

fn parse_ts(value: &str) -> rusqlite::Result<DateTime<Utc>> {
    use chrono::NaiveDateTime;
    let naive = NaiveDateTime::parse_from_str(value, DATETIME_FORMAT).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let local = BEIJING_OFFSET
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| rusqlite::Error::ExecuteReturnedResults)?;
    Ok(local.with_timezone(&Utc))
}
