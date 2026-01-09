use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;

#[derive(Debug, Clone)]
pub struct PasswordResetTokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait PasswordResetTokenStore: Send + Sync {
    async fn create_password_reset_token(
        &self,
        token: PasswordResetTokenRecord,
    ) -> Result<(), GatewayError>;

    async fn has_recent_active_password_reset_token(
        &self,
        user_id: &str,
        since: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, GatewayError>;

    async fn consume_password_reset_token(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PasswordResetTokenRecord>, GatewayError>;
}

pub fn issue_password_reset_token() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64_URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand::Rng;

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    B64_URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_password_reset_token(token: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

