use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;
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
        Ok(Some(PasswordResetTokenRecord {
            id: row.get(0),
            user_id: row.get(1),
            token_hash: row.get(2),
            created_at: row.get::<usize, DateTime<Utc>>(3),
            expires_at: row.get::<usize, DateTime<Utc>>(4),
            used_at: row.get(5),
        }))
    }
}

