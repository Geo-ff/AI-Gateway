use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;
use crate::refresh_tokens::{RefreshTokenRecord, RefreshTokenStore};

#[async_trait]
impl RefreshTokenStore for PgLogStore {
    async fn create_refresh_token(&self, token: RefreshTokenRecord) -> Result<(), GatewayError> {
        let client = self.pool.pick();
        client
            .execute(
                "INSERT INTO refresh_tokens (id, user_id, token_hash, created_at, expires_at, revoked_at, replaced_by_id, last_used_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
                &[
                    &token.id,
                    &token.user_id,
                    &token.token_hash,
                    &token.created_at,
                    &token.expires_at,
                    &token.revoked_at,
                    &token.replaced_by_id,
                    &token.last_used_at,
                ],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }

    async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRecord>, GatewayError> {
        let client = self.pool.pick();
        let row_opt = client
            .query_opt(
                "SELECT id, user_id, token_hash, created_at, expires_at, revoked_at, replaced_by_id, last_used_at
                 FROM refresh_tokens
                 WHERE token_hash = $1
                 LIMIT 1",
                &[&token_hash],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(row) = row_opt else {
            return Ok(None);
        };
        Ok(Some(RefreshTokenRecord {
            id: row.get(0),
            user_id: row.get(1),
            token_hash: row.get(2),
            created_at: row.get::<usize, DateTime<Utc>>(3),
            expires_at: row.get::<usize, DateTime<Utc>>(4),
            revoked_at: row.get(5),
            replaced_by_id: row.get(6),
            last_used_at: row.get(7),
        }))
    }

    async fn revoke_refresh_token(
        &self,
        token_hash: &str,
        when: DateTime<Utc>,
    ) -> Result<bool, GatewayError> {
        let client = self.pool.pick();
        let changed = client
            .execute(
                "UPDATE refresh_tokens
                 SET revoked_at = COALESCE(revoked_at, $2),
                     last_used_at = COALESCE(last_used_at, $2)
                 WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&token_hash, &when],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(changed > 0)
    }

    async fn revoke_all_refresh_tokens_for_user(
        &self,
        user_id: &str,
        when: DateTime<Utc>,
    ) -> Result<u64, GatewayError> {
        let client = self.pool.pick();
        let changed = client
            .execute(
                "UPDATE refresh_tokens
                 SET revoked_at = COALESCE(revoked_at, $2),
                     last_used_at = COALESCE(last_used_at, $2)
                 WHERE user_id = $1 AND revoked_at IS NULL",
                &[&user_id, &when],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(changed)
    }

    async fn set_refresh_token_replaced_by(
        &self,
        token_hash: &str,
        replaced_by_id: &str,
    ) -> Result<(), GatewayError> {
        let client = self.pool.pick();
        let _ = client
            .execute(
                "UPDATE refresh_tokens SET replaced_by_id = $2 WHERE token_hash = $1",
                &[&token_hash, &replaced_by_id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }
}
