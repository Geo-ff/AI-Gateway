use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::{Signature, VerifyingKey};
use hex::encode as hex_encode;
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::error::GatewayError;
use crate::server::storage_traits::{
    AdminPublicKeyRecord, LoginCodeRecord, LoginStore, TuiSessionRecord, WebSessionRecord,
};

const CODE_COOLDOWN_SECS: i64 = 5;
const TUI_SESSION_TTL_HOURS: i64 = 12;
const WEB_SESSION_TTL_HOURS: i64 = 8;
const CHALLENGE_TTL_SECS: i64 = 120;
const SIGNING_PREFIX: &[u8] = b"gateway-auth:";
const CHALLENGE_NONCE_LEN: usize = 32;
const TUI_TOKEN_LEN: usize = 64;
const WEB_SESSION_ID_LEN: usize = 56;

#[derive(Debug, Clone)]
pub struct LoginCodeEntry {
    pub code: String,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub uses: u32,
    #[allow(dead_code)]
    pub disabled: bool,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct LoginCodeStatus {
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub uses: u32,
    pub remaining_uses: u32,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: String,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub expires_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TuiSession {
    pub token: String,
    pub fingerprint: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TuiChallenge {
    pub challenge_id: String,
    pub nonce_b64: String,
    pub expires_at: DateTime<Utc>,
}

struct ChallengeEntry {
    fingerprint: String,
    public_key: Vec<u8>,
    nonce: Vec<u8>,
    expires_at: DateTime<Utc>,
}

pub struct LoginManager {
    store: Arc<dyn LoginStore + Send + Sync>,
    challenges: Arc<RwLock<HashMap<String, ChallengeEntry>>>,
}

impl LoginManager {
    pub fn new(store: Arc<dyn LoginStore + Send + Sync>) -> Self {
        Self {
            store,
            challenges: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn list_admin_keys(&self) -> Result<Vec<AdminPublicKeyRecord>, GatewayError> {
        self.store.list_admin_keys().await.map_err(GatewayError::Db)
    }

    pub async fn add_admin_key(&self, record: &AdminPublicKeyRecord) -> Result<(), GatewayError> {
        self.store
            .insert_admin_key(record)
            .await
            .map_err(GatewayError::Db)
    }

    pub async fn delete_admin_key(&self, fingerprint: &str) -> Result<bool, GatewayError> {
        self.store
            .delete_admin_key(fingerprint)
            .await
            .map_err(GatewayError::Db)
    }

    pub async fn list_tui_sessions(
        &self,
        fingerprint: Option<&str>,
    ) -> Result<Vec<TuiSessionRecord>, GatewayError> {
        self.store
            .list_tui_sessions(fingerprint)
            .await
            .map_err(GatewayError::Db)
    }

    fn random_string(len: usize) -> String {
        let rng = rand::rng();
        use rand::distr::Alphanumeric;
        rng.sample_iter(&Alphanumeric)
            .take(len)
            .map(char::from)
            .collect()
    }

    fn hash_code(code: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        hex_encode(hasher.finalize())
    }

    pub fn fingerprint_for_public_key(public_key: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(public_key);
        hex_encode(hasher.finalize())
    }

    async fn prune_challenges(&self) {
        let now = Utc::now();
        let mut guard = self.challenges.write().await;
        guard.retain(|_, entry| entry.expires_at > now);
    }

    async fn load_admin_key(
        &self,
        fingerprint: &str,
    ) -> Result<AdminPublicKeyRecord, GatewayError> {
        let key = self
            .store
            .get_admin_key(fingerprint)
            .await
            .map_err(GatewayError::Db)?;
        let Some(key) = key else {
            return Err(GatewayError::Config("管理员公钥不存在或未注册".into()));
        };
        if !key.enabled {
            return Err(GatewayError::Config("管理员公钥已禁用".into()));
        }
        Ok(key)
    }

    pub async fn issue_challenge(&self, fingerprint: &str) -> Result<TuiChallenge, GatewayError> {
        self.prune_challenges().await;
        let key = self.load_admin_key(fingerprint).await?;
        if key.public_key.len() != ed25519_dalek::PUBLIC_KEY_LENGTH {
            return Err(GatewayError::Config("管理员公钥长度异常".into()));
        }
        let mut nonce = vec![0u8; CHALLENGE_NONCE_LEN];
        rand::rng().fill(&mut nonce[..]);
        let challenge_id = Self::random_string(48);
        let expires_at = Utc::now() + Duration::seconds(CHALLENGE_TTL_SECS);
        {
            let mut guard = self.challenges.write().await;
            guard.insert(
                challenge_id.clone(),
                ChallengeEntry {
                    fingerprint: fingerprint.to_string(),
                    public_key: key.public_key.clone(),
                    nonce: nonce.clone(),
                    expires_at,
                },
            );
        }
        Ok(TuiChallenge {
            challenge_id,
            nonce_b64: B64_STANDARD.encode(nonce),
            expires_at,
        })
    }

    pub async fn verify_challenge(
        &self,
        challenge_id: &str,
        fingerprint: &str,
        signature_b64: &str,
    ) -> Result<TuiSession, GatewayError> {
        let challenge = {
            let mut guard = self.challenges.write().await;
            guard.remove(challenge_id)
        };
        let Some(challenge) = challenge else {
            return Err(GatewayError::Config("挑战不存在或已过期".into()));
        };
        if challenge.fingerprint != fingerprint {
            return Err(GatewayError::Config("挑战与指纹不匹配".into()));
        }
        if Utc::now() > challenge.expires_at {
            return Err(GatewayError::Config("挑战已过期".into()));
        }
        let pub_bytes: [u8; ed25519_dalek::PUBLIC_KEY_LENGTH] = challenge
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| GatewayError::Config("管理员公钥长度异常".into()))?;
        let verifying_key = VerifyingKey::from_bytes(&pub_bytes)
            .map_err(|_| GatewayError::Config("管理员公钥解析失败".into()))?;

        let sig_raw = B64_STANDARD
            .decode(signature_b64)
            .map_err(|_| GatewayError::Config("签名格式错误".into()))?;
        let sig_bytes: [u8; ed25519_dalek::SIGNATURE_LENGTH] = sig_raw
            .as_slice()
            .try_into()
            .map_err(|_| GatewayError::Config("签名长度错误".into()))?;
        let signature = Signature::from_bytes(&sig_bytes);

        let mut message = Vec::with_capacity(SIGNING_PREFIX.len() + challenge.nonce.len());
        message.extend_from_slice(SIGNING_PREFIX);
        message.extend_from_slice(&challenge.nonce);
        verifying_key
            .verify_strict(&message, &signature)
            .map_err(|_| GatewayError::Config("签名验证失败".into()))?;

        let now = Utc::now();
        let expires_at = now + Duration::hours(TUI_SESSION_TTL_HOURS);
        let token = Self::random_string(TUI_TOKEN_LEN);
        let session = TuiSessionRecord {
            session_id: token.clone(),
            fingerprint: fingerprint.to_string(),
            issued_at: now,
            expires_at,
            revoked: false,
            last_code_at: None,
        };
        self.store
            .create_tui_session(&session)
            .await
            .map_err(GatewayError::Db)?;
        self.store
            .touch_admin_key(fingerprint, now)
            .await
            .map_err(GatewayError::Db)?;
        Ok(TuiSession {
            token,
            fingerprint: fingerprint.to_string(),
            expires_at,
        })
    }

    pub async fn validate_tui_token(
        &self,
        token: &str,
    ) -> Result<Option<TuiSessionRecord>, GatewayError> {
        if token.is_empty() {
            return Ok(None);
        }
        let session = self
            .store
            .get_tui_session(token)
            .await
            .map_err(GatewayError::Db)?;
        let Some(session) = session else {
            return Ok(None);
        };
        if session.revoked {
            return Ok(None);
        }
        if Utc::now() > session.expires_at {
            let _ = self
                .store
                .revoke_tui_session(token)
                .await
                .map_err(GatewayError::Db)?;
            return Ok(None);
        }
        Ok(Some(session))
    }

    pub async fn revoke_tui_session(&self, token: &str) -> Result<bool, GatewayError> {
        self.store
            .revoke_tui_session(token)
            .await
            .map_err(GatewayError::Db)
    }

    pub async fn generate_code(
        &self,
        session: &TuiSessionRecord,
        ttl_secs: u64,
        max_uses: u32,
        length: usize,
    ) -> Result<LoginCodeEntry, GatewayError> {
        let now = Utc::now();
        if let Some(last) = session.last_code_at
            && now - last < Duration::seconds(CODE_COOLDOWN_SECS)
        {
            return Err(GatewayError::RateLimited("生成频率过快，请稍后再试".into()));
        }
        self.store
            .disable_codes_for_session(&session.session_id)
            .await
            .map_err(GatewayError::Db)?;
        let mut code = String::new();
        while code.is_empty() {
            let candidate = Self::random_string(length);
            if candidate.chars().all(|c| c.is_ascii_alphanumeric()) {
                code = candidate;
            }
        }
        let expires_at = now + Duration::seconds(ttl_secs as i64);
        let hint = if code.len() >= 6 {
            Some(format!("{}{}", &code[0..3], &code[code.len() - 3..]))
        } else {
            None
        };
        let record = LoginCodeRecord {
            code_hash: Self::hash_code(&code),
            session_id: session.session_id.clone(),
            fingerprint: session.fingerprint.clone(),
            created_at: now,
            expires_at,
            max_uses: max_uses.max(1),
            uses: 0,
            disabled: false,
            hint,
        };
        self.store
            .insert_login_code(&record)
            .await
            .map_err(GatewayError::Db)?;
        self.store
            .update_tui_session_last_code(&session.session_id, now)
            .await
            .map_err(GatewayError::Db)?;
        Ok(LoginCodeEntry {
            code,
            created_at: now,
            expires_at,
            max_uses: record.max_uses,
            uses: 0,
            disabled: false,
        })
    }

    pub async fn current_code_status(
        &self,
        session: &TuiSessionRecord,
    ) -> Result<Option<LoginCodeStatus>, GatewayError> {
        let record = self
            .store
            .get_latest_login_code_for_session(&session.session_id)
            .await
            .map_err(GatewayError::Db)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let expired = Utc::now() > record.expires_at;
        let mut remaining = record.max_uses.saturating_sub(record.uses);
        if record.disabled || expired {
            remaining = 0;
        }
        Ok(Some(LoginCodeStatus {
            created_at: record.created_at,
            expires_at: record.expires_at,
            max_uses: record.max_uses,
            uses: record.uses,
            remaining_uses: remaining,
            disabled: record.disabled || expired,
        }))
    }

    pub async fn redeem(&self, code: &str) -> Result<Option<SessionEntry>, GatewayError> {
        let now = Utc::now();
        let hash = Self::hash_code(code);
        let record = self
            .store
            .redeem_login_code(&hash, now)
            .await
            .map_err(GatewayError::Db)?;
        let Some(record) = record else {
            return Ok(None);
        };
        let session_id = Self::random_string(WEB_SESSION_ID_LEN);
        let expires_at = now + Duration::hours(WEB_SESSION_TTL_HOURS);
        let web_record = WebSessionRecord {
            session_id: session_id.clone(),
            fingerprint: Some(record.fingerprint.clone()),
            created_at: now,
            expires_at,
            revoked: false,
            issued_by_code: Some(record.code_hash.clone()),
        };
        self.store
            .insert_web_session(&web_record)
            .await
            .map_err(GatewayError::Db)?;
        Ok(Some(SessionEntry {
            id: session_id,
            created_at: now,
            expires_at,
            fingerprint: Some(record.fingerprint),
        }))
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<SessionEntry>, GatewayError> {
        if id.is_empty() {
            return Ok(None);
        }
        let record = self
            .store
            .get_web_session(id)
            .await
            .map_err(GatewayError::Db)?;
        let Some(record) = record else {
            return Ok(None);
        };
        if record.revoked {
            return Ok(None);
        }
        if Utc::now() > record.expires_at {
            let _ = self
                .store
                .revoke_web_session(id)
                .await
                .map_err(GatewayError::Db)?;
            return Ok(None);
        }
        Ok(Some(SessionEntry {
            id: record.session_id,
            created_at: record.created_at,
            expires_at: record.expires_at,
            fingerprint: record.fingerprint,
        }))
    }

    pub async fn revoke_session(&self, id: &str) -> Result<bool, GatewayError> {
        self.store
            .revoke_web_session(id)
            .await
            .map_err(GatewayError::Db)
    }
}
