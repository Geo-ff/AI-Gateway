use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;

#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub replaced_by_id: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait RefreshTokenStore: Send + Sync {
    async fn create_refresh_token(&self, token: RefreshTokenRecord) -> Result<(), GatewayError>;

    async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRecord>, GatewayError>;

    async fn revoke_refresh_token(
        &self,
        token_hash: &str,
        when: DateTime<Utc>,
    ) -> Result<bool, GatewayError>;

    async fn revoke_all_refresh_tokens_for_user(
        &self,
        user_id: &str,
        when: DateTime<Utc>,
    ) -> Result<u64, GatewayError>;

    async fn set_refresh_token_replaced_by(
        &self,
        token_hash: &str,
        replaced_by_id: &str,
    ) -> Result<(), GatewayError>;
}

pub fn refresh_ttl_secs() -> i64 {
    std::env::var("GW_REFRESH_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30 * 24 * 60 * 60)
}

pub fn issue_refresh_token() -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64_URL_SAFE_NO_PAD;
    use rand::Rng;

    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    B64_URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_refresh_token(token: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
