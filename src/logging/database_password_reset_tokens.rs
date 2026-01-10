use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;

use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::password_reset_tokens::{PasswordResetTokenRecord, PasswordResetTokenStore};

fn row_to_password_reset_token(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PasswordResetTokenRecord> {
    let created_at_s: String = row.get(3)?;
    let expires_at_s: String = row.get(4)?;
    let used_at_s: Option<String> = row.get(5)?;
    Ok(PasswordResetTokenRecord {
        id: row.get(0)?,
        user_id: row.get(1)?,
        token_hash: row.get(2)?,
        created_at: parse_beijing_string(&created_at_s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        expires_at: parse_beijing_string(&expires_at_s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        used_at: used_at_s
            .as_deref()
            .map(parse_beijing_string)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?,
    })
}

#[async_trait]
impl PasswordResetTokenStore for DatabaseLogger {
    async fn create_password_reset_token(
        &self,
        token: PasswordResetTokenRecord,
    ) -> Result<(), GatewayError> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO password_reset_tokens (id, user_id, token_hash, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                token.id,
                token.user_id,
                token.token_hash,
                to_beijing_string(&token.created_at),
                to_beijing_string(&token.expires_at),
                token.used_at.map(|v| to_beijing_string(&v)),
            ],
        )?;
        Ok(())
    }

    async fn has_recent_active_password_reset_token(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let row = conn
            .query_row(
                "SELECT 1
                 FROM password_reset_tokens
                 WHERE user_id = ?1
                   AND created_at >= ?2
                   AND used_at IS NULL
                   AND expires_at > ?3
                 LIMIT 1",
                rusqlite::params![user_id, to_beijing_string(&since), to_beijing_string(&now)],
                |_row| Ok(1i64),
            )
            .optional()?;
        Ok(row.is_some())
    }

    async fn consume_password_reset_token(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PasswordResetTokenRecord>, GatewayError> {
        let mut conn = self.connection.lock().await;
        let tx = conn.transaction()?;

        let changed = tx.execute(
            "UPDATE password_reset_tokens
             SET used_at = COALESCE(used_at, ?2)
             WHERE token_hash = ?1
               AND used_at IS NULL
               AND expires_at > ?2",
            rusqlite::params![token_hash, to_beijing_string(&now)],
        )?;
        if changed == 0 {
            tx.commit()?;
            return Ok(None);
        }

        let record_opt = tx
            .query_row(
                "SELECT id, user_id, token_hash, created_at, expires_at, used_at
                 FROM password_reset_tokens
                 WHERE token_hash = ?1
                 LIMIT 1",
                [token_hash],
                row_to_password_reset_token,
            )
            .optional()?;
        tx.commit()?;
        Ok(record_opt)
    }
}
