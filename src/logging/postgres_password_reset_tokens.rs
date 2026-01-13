use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;
use crate::logging::time::parse_datetime_string;
use crate::password_reset_tokens::{PasswordResetTokenRecord, PasswordResetTokenStore};

#[async_trait]
impl PasswordResetTokenStore for PgLogStore {
    async fn create_password_reset_token(
        &self,
        token: PasswordResetTokenRecord,
    ) -> Result<(), GatewayError> {
        let client = self.pool.pick();
        client
            .execute(
                "INSERT INTO password_reset_tokens (id, user_id, token_hash, created_at, expires_at, used_at)
                 VALUES ($1,$2,$3,$4,$5,$6)",
                &[
                    &token.id,
                    &token.user_id,
                    &token.token_hash,
                    &token.created_at,
                    &token.expires_at,
                    &token.used_at,
                ],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }

    async fn has_recent_active_password_reset_token(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, GatewayError> {
        let client = self.pool.pick();
        let row = client
            .query_opt(
                "SELECT 1
                 FROM password_reset_tokens
                 WHERE user_id = $1
                   AND created_at >= $2
                   AND used_at IS NULL
                   AND expires_at > $3
                 LIMIT 1",
                &[&user_id, &since, &now],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(row.is_some())
    }

    async fn consume_password_reset_token(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PasswordResetTokenRecord>, GatewayError> {
        let client = self.pool.pick();
        let row_opt = client
            .query_opt(
                "UPDATE password_reset_tokens
                 SET used_at = COALESCE(used_at, $2)
                 WHERE token_hash = $1
                   AND used_at IS NULL
                   AND expires_at > $2
                 RETURNING id, user_id, token_hash, created_at, expires_at, used_at",
                &[&token_hash, &now],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(row) = row_opt else {
            return Ok(None);
        };
        let created_at = if let Ok(dt) = row.try_get::<usize, DateTime<Utc>>(3) {
            dt
        } else if let Ok(raw) = row.try_get::<usize, String>(3) {
            parse_datetime_string(&raw).unwrap_or(now)
        } else {
            now
        };
        let expires_at = if let Ok(dt) = row.try_get::<usize, DateTime<Utc>>(4) {
            dt
        } else if let Ok(raw) = row.try_get::<usize, String>(4) {
            parse_datetime_string(&raw).unwrap_or(now)
        } else {
            now
        };
        let used_at = if let Ok(dt) = row.try_get::<usize, Option<DateTime<Utc>>>(5) {
            dt
        } else if let Ok(raw) = row.try_get::<usize, Option<String>>(5) {
            raw.and_then(|s| parse_datetime_string(&s).ok())
        } else if let Ok(raw) = row.try_get::<usize, String>(5) {
            parse_datetime_string(&raw).ok()
        } else {
            None
        };
        Ok(Some(PasswordResetTokenRecord {
            id: row.try_get(0).unwrap_or_default(),
            user_id: row.try_get(1).unwrap_or_default(),
            token_hash: row.try_get(2).unwrap_or_default(),
            created_at,
            expires_at,
            used_at,
        }))
    }
}
