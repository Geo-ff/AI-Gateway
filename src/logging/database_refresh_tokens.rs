use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;

use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::refresh_tokens::{RefreshTokenRecord, RefreshTokenStore};

fn row_to_refresh_token(row: &rusqlite::Row<'_>) -> rusqlite::Result<RefreshTokenRecord> {
    let created_at_s: String = row.get(3)?;
    let expires_at_s: String = row.get(4)?;
    let revoked_at_s: Option<String> = row.get(5)?;
    let last_used_at_s: Option<String> = row.get(7)?;
    Ok(RefreshTokenRecord {
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
        revoked_at: revoked_at_s
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
        replaced_by_id: row.get(6)?,
        last_used_at: last_used_at_s
            .as_deref()
            .map(parse_beijing_string)
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                )
            })?,
    })
}

#[async_trait]
impl RefreshTokenStore for DatabaseLogger {
    async fn create_refresh_token(&self, token: RefreshTokenRecord) -> Result<(), GatewayError> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO refresh_tokens (id, user_id, token_hash, created_at, expires_at, revoked_at, replaced_by_id, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                token.id,
                token.user_id,
                token.token_hash,
                to_beijing_string(&token.created_at),
                to_beijing_string(&token.expires_at),
                token.revoked_at.map(|v| to_beijing_string(&v)),
                token.replaced_by_id,
                token.last_used_at.map(|v| to_beijing_string(&v)),
            ],
        )?;
        Ok(())
    }

    async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRecord>, GatewayError> {
        let conn = self.connection.lock().await;
        let row = conn
            .query_row(
                "SELECT id, user_id, token_hash, created_at, expires_at, revoked_at, replaced_by_id, last_used_at
                 FROM refresh_tokens
                 WHERE token_hash = ?1
                 LIMIT 1",
                [token_hash],
                row_to_refresh_token,
            )
            .optional()?;
        Ok(row)
    }

    async fn revoke_refresh_token(
        &self,
        token_hash: &str,
        when: DateTime<Utc>,
    ) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let changed = conn.execute(
            "UPDATE refresh_tokens
             SET revoked_at = COALESCE(revoked_at, ?2),
                 last_used_at = COALESCE(last_used_at, ?2)
             WHERE token_hash = ?1 AND revoked_at IS NULL",
            rusqlite::params![token_hash, to_beijing_string(&when)],
        )?;
        Ok(changed > 0)
    }

    async fn revoke_all_refresh_tokens_for_user(
        &self,
        user_id: &str,
        when: DateTime<Utc>,
    ) -> Result<u64, GatewayError> {
        let conn = self.connection.lock().await;
        let changed = conn.execute(
            "UPDATE refresh_tokens
             SET revoked_at = COALESCE(revoked_at, ?2),
                 last_used_at = COALESCE(last_used_at, ?2)
             WHERE user_id = ?1 AND revoked_at IS NULL",
            rusqlite::params![user_id, to_beijing_string(&when)],
        )?;
        Ok(changed as u64)
    }

    async fn set_refresh_token_replaced_by(
        &self,
        token_hash: &str,
        replaced_by_id: &str,
    ) -> Result<(), GatewayError> {
        let conn = self.connection.lock().await;
        let _ = conn.execute(
            "UPDATE refresh_tokens SET replaced_by_id = ?2 WHERE token_hash = ?1",
            rusqlite::params![token_hash, replaced_by_id],
        )?;
        Ok(())
    }
}
