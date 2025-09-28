use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone)]
pub struct LoginCodeEntry {
    pub code: String,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub uses: u32,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct LoginManager {
    codes: Arc<RwLock<HashMap<String, LoginCodeEntry>>>,
    sessions: Arc<RwLock<HashMap<String, SessionEntry>>>,
}

impl LoginManager {
    pub fn new() -> Self {
        Self {
            codes: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn generate_code(&self, ttl_secs: u64, max_uses: u32, length: usize) -> LoginCodeEntry {
        use rand::Rng;
        let mut code = String::new();
        while code.is_empty() || self.codes.read().await.contains_key(&code) {
            let mut s = String::new();
            let rng = rand::rng();
            use rand::distr::Alphanumeric;
            s.extend(rng.sample_iter(&Alphanumeric).take(length).map(char::from));
            code = s;
        }
        let now = Utc::now();
        let entry = LoginCodeEntry {
            code: code.clone(),
            created_at: now,
            expires_at: now + Duration::seconds(ttl_secs as i64),
            max_uses,
            uses: 0,
            disabled: false,
        };
        self.codes.write().await.insert(code.clone(), entry.clone());
        entry
    }

    pub async fn redeem(&self, code: &str) -> Option<SessionEntry> {
        let mut codes = self.codes.write().await;
        let Some(entry) = codes.get_mut(code) else { return None; };
        if entry.disabled { return None; }
        if Utc::now() > entry.expires_at { entry.disabled = true; return None; }
        if entry.uses >= entry.max_uses { entry.disabled = true; return None; }
        entry.uses += 1;
        if entry.uses >= entry.max_uses { entry.disabled = true; }
        // create session
        let sess = self.create_session(Duration::hours(8)).await;
        Some(sess)
    }

    pub async fn create_session(&self, ttl: Duration) -> SessionEntry {
        use rand::Rng;
        let id: String = {
            let rng = rand::rng();
            use rand::distr::Alphanumeric;
            rng.sample_iter(&Alphanumeric).take(56).map(char::from).collect()
        };
        let now = Utc::now();
        let sess = SessionEntry { id: id.clone(), created_at: now, expires_at: now + ttl };
        self.sessions.write().await.insert(id.clone(), sess.clone());
        sess
    }

    pub async fn get_session(&self, id: &str) -> Option<SessionEntry> {
        self.sessions.read().await.get(id).cloned().filter(|s| Utc::now() <= s.expires_at)
    }

    pub async fn revoke_session(&self, id: &str) -> bool {
        self.sessions.write().await.remove(id).is_some()
    }
}

