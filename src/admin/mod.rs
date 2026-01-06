use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;
use crate::logging::time::{parse_beijing_string, to_beijing_string};

const ADMIN_TOKEN_ID_PREFIX: &str = "atk_";

pub(crate) fn admin_token_id_for_token(token: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let hex = hex::encode(hasher.finalize());
    format!("{}{}", ADMIN_TOKEN_ID_PREFIX, &hex[..24])
}

pub(crate) fn normalize_admin_token_name(name: Option<String>, id: &str) -> String {
    let trimmed = name.map(|v| v.trim().to_string());
    if let Some(v) = trimmed.filter(|v| !v.is_empty()) {
        return v;
    }
    // Default: stable, non-sensitive, human-friendly
    let suffix = id.strip_prefix(ADMIN_TOKEN_ID_PREFIX).unwrap_or(id);
    format!("token-{}", &suffix[..8.min(suffix.len())])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminToken {
    pub id: String,
    pub name: String,
    pub token: String,
    pub allowed_models: Option<Vec<String>>, // None 表示不限制
    pub max_tokens: Option<i64>,             // 兼容旧字段（不再使用）
    pub max_amount: Option<f64>,             // 金额额度（单位自定义，如 USD/CNY）
    pub enabled: bool,
    pub expires_at: Option<DateTime<Utc>>, // None 表示不过期
    pub created_at: DateTime<Utc>,
    pub amount_spent: f64,            // 累计消费金额（默认 0）
    pub prompt_tokens_spent: i64,     // 累计提示/输入 tokens
    pub completion_tokens_spent: i64, // 累计补全/回复 tokens
    pub total_tokens_spent: i64,      // 累计总 tokens
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTokenPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub token: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>, // None 表示不限制
    #[serde(default)]
    pub max_tokens: Option<i64>, // 兼容旧字段（忽略）
    #[serde(default)]
    pub max_amount: Option<f64>, // 金额额度（可选）
    #[serde(default = "default_enabled_true")]
    pub enabled: bool,
    #[serde(default)]
    pub expires_at: Option<String>, // 北京时间字符串，可选
}

fn default_enabled_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTokenPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<Vec<String>>, // None -> 不修改；Some(Some(vec)) -> 设置；Some(None) -> 清空
    #[serde(default)]
    pub max_tokens: Option<Option<i64>>, // 兼容旧字段（忽略）
    #[serde(default)]
    pub max_amount: Option<Option<f64>>, // None -> 不修改；Some(Some(v)) -> 设置；Some(None) -> 清空
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub expires_at: Option<Option<String>>, // None -> 不修改；Some(Some(s)) -> 设置；Some(None) -> 清空
}

#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn create_token(&self, payload: CreateTokenPayload) -> Result<AdminToken, GatewayError>;
    async fn update_token(
        &self,
        token: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError>;
    async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, GatewayError>;
    async fn get_token(&self, token: &str) -> Result<Option<AdminToken>, GatewayError>;
    async fn get_token_by_id(&self, id: &str) -> Result<Option<AdminToken>, GatewayError>;
    async fn list_tokens(&self) -> Result<Vec<AdminToken>, GatewayError>;
    async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), GatewayError>;
    async fn add_usage_spent(
        &self,
        token: &str,
        prompt: i64,
        completion: i64,
        total: i64,
    ) -> Result<(), GatewayError>;
    async fn delete_token(&self, token: &str) -> Result<bool, GatewayError>;
    async fn delete_token_by_id(&self, id: &str) -> Result<bool, GatewayError>;
    async fn update_token_by_id(
        &self,
        id: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError>;
    async fn set_enabled_by_id(&self, id: &str, enabled: bool) -> Result<bool, GatewayError>;
}

// SQLite 的实现由 DatabaseLogger 提供（见 logging/database_admin_tokens.rs）

// ------------------ Postgres 实现（GaussDB 兼容） ------------------

pub struct PgTokenStore {
    client: std::sync::Arc<tokio_postgres::Client>,
}

// --- helpers to keep Postgres mapping concise and consistent ---
fn join_allowed_models(v: &Option<Vec<String>>) -> Option<String> {
    v.as_ref().map(|list| list.join(","))
}

fn parse_allowed_models(s: Option<String>) -> Option<Vec<String>> {
    s.map(|v| {
        v.split(',')
            .filter(|x| !x.trim().is_empty())
            .map(|x| x.trim().to_string())
            .collect::<Vec<_>>()
    })
    .and_then(|v| if v.is_empty() { None } else { Some(v) })
}

fn row_to_admin_token(r: &tokio_postgres::Row) -> AdminToken {
    let id_opt: Option<String> = r.get(0);
    let name_opt: Option<String> = r.get(1);
    let token: String = r.get(2);
    let allowed_s: Option<String> = r.get(3);
    let max_tokens: Option<i64> = r.get(4);
    let enabled: bool = r.get::<usize, Option<bool>>(5).unwrap_or(true);
    let expires_s: Option<String> = r.get(6);
    let created_s: String = r.get(7);
    let max_amount: Option<f64> = r.get(8);
    let amount_spent: f64 = r.get::<usize, Option<f64>>(9).unwrap_or(0.0);
    let prompt_tokens_spent: i64 = r.get::<usize, Option<i64>>(10).unwrap_or(0);
    let completion_tokens_spent: i64 = r.get::<usize, Option<i64>>(11).unwrap_or(0);
    let total_tokens_spent: i64 = r.get::<usize, Option<i64>>(12).unwrap_or(0);
    let id = id_opt.unwrap_or_else(|| admin_token_id_for_token(&token));
    let name = normalize_admin_token_name(name_opt, &id);
    AdminToken {
        id,
        name,
        token,
        allowed_models: parse_allowed_models(allowed_s),
        max_tokens,
        max_amount,
        enabled,
        expires_at: expires_s.and_then(|s| parse_beijing_string(&s).ok()),
        created_at: parse_beijing_string(&created_s).unwrap_or(Utc::now()),
        amount_spent,
        prompt_tokens_spent,
        completion_tokens_spent,
        total_tokens_spent,
    }
}

impl PgTokenStore {
    pub async fn connect(pg_url: &str, schema: Option<&str>) -> Result<Self, GatewayError> {
        let (client, connection) = tokio_postgres::connect(pg_url, tokio_postgres::NoTls)
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to connect postgres: {}", e)))?;
        // spawn connection task
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("postgres connection error: {}", e);
            }
        });
        if let Some(s) = schema {
            client
                .execute(&format!("SET search_path TO {}", s), &[])
                .await
                .map_err(|e| GatewayError::Config(format!("Failed to set search_path: {}", e)))?;
        }
        client
            .execute(
                r#"CREATE TABLE IF NOT EXISTS admin_tokens (
                id TEXT UNIQUE,
                name TEXT,
                token TEXT PRIMARY KEY,
                allowed_models TEXT,
                max_tokens BIGINT,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                expires_at TEXT,
                created_at TEXT NOT NULL,
                max_amount DOUBLE PRECISION,
                amount_spent DOUBLE PRECISION DEFAULT 0,
                prompt_tokens_spent BIGINT DEFAULT 0,
                completion_tokens_spent BIGINT DEFAULT 0,
                total_tokens_spent BIGINT DEFAULT 0
            )"#,
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("Failed to init admin_tokens: {}", e)))?;
        // Migration
        let _ = client.execute("ALTER TABLE admin_tokens ADD COLUMN id TEXT", &[]).await;
        let _ = client.execute("ALTER TABLE admin_tokens ADD COLUMN name TEXT", &[]).await;
        let _ = client
            .execute(
                "ALTER TABLE admin_tokens ADD COLUMN max_amount DOUBLE PRECISION",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "ALTER TABLE admin_tokens ADD COLUMN amount_spent DOUBLE PRECISION DEFAULT 0",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "ALTER TABLE admin_tokens ADD COLUMN prompt_tokens_spent BIGINT DEFAULT 0",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "ALTER TABLE admin_tokens ADD COLUMN completion_tokens_spent BIGINT DEFAULT 0",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "ALTER TABLE admin_tokens ADD COLUMN total_tokens_spent BIGINT DEFAULT 0",
                &[],
            )
            .await;
        let _ = client
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS admin_tokens_id_uidx ON admin_tokens(id)",
                &[],
            )
            .await;

        // Backfill id/name for existing rows (best-effort)
        if let Ok(rows) = client
            .query(
                "SELECT token FROM admin_tokens WHERE id IS NULL OR id = '' OR name IS NULL OR name = ''",
                &[],
            )
            .await
        {
            for r in rows {
                let tok: String = r.get(0);
                let id = admin_token_id_for_token(&tok);
                let name = normalize_admin_token_name(None, &id);
                let _ = client
                    .execute(
                        "UPDATE admin_tokens SET id = $2 WHERE token = $1 AND (id IS NULL OR id = '')",
                        &[&tok, &id],
                    )
                    .await;
                let _ = client
                    .execute(
                        "UPDATE admin_tokens SET name = $2 WHERE token = $1 AND (name IS NULL OR name = '')",
                        &[&tok, &name],
                    )
                    .await;
            }
        }
        let store = Self {
            client: std::sync::Arc::new(client),
        };
        // keepalive（带抖动），降低空闲回收的概率并避免集群齐刷刷触发
        crate::db::postgres::spawn_keepalive(std::sync::Arc::clone(&store.client), 240, 420);
        Ok(store)
    }
}

#[async_trait]
impl TokenStore for PgTokenStore {
    async fn create_token(&self, payload: CreateTokenPayload) -> Result<AdminToken, GatewayError> {
        // 始终生成随机令牌，忽略传入 token 字段
        let token = {
            use rand::Rng;
            let rng = rand::rng();
            use rand::distr::Alphanumeric;
            rng.sample_iter(&Alphanumeric)
                .take(40)
                .map(char::from)
                .collect::<String>()
        };
        let id = admin_token_id_for_token(&token);
        let name = normalize_admin_token_name(payload.name.clone(), &id);
        let now = Utc::now();
        let allowed_models_s = join_allowed_models(&payload.allowed_models);
        let expires_s = payload.expires_at.clone();
        self.client
            .execute(
                "INSERT INTO admin_tokens (id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, 0, 0)",
                &[&id, &name, &token, &allowed_models_s, &payload.max_tokens, &payload.enabled, &expires_s, &to_beijing_string(&now), &payload.max_amount],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(AdminToken {
            id,
            name,
            token,
            allowed_models: payload.allowed_models,
            max_tokens: payload.max_tokens,
            max_amount: payload.max_amount,
            enabled: payload.enabled,
            expires_at: match expires_s {
                Some(s) => Some(parse_beijing_string(&s)?),
                None => None,
            },
            created_at: now,
            amount_spent: 0.0,
            prompt_tokens_spent: 0,
            completion_tokens_spent: 0,
            total_tokens_spent: 0,
        })
    }

    async fn update_token(
        &self,
        token: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError> {
        // read existing
        let row = self.client
            .query_opt(
                "SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE token = $1",
                &[&token],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(r) = row else { return Ok(None) };
        let mut current = row_to_admin_token(&r);

        if let Some(v) = payload.name {
            current.name = normalize_admin_token_name(Some(v), &current.id);
        }
        if let Some(v) = payload.allowed_models {
            current.allowed_models = Some(v);
        }
        if let Some(v) = payload.max_tokens {
            current.max_tokens = v;
        }
        if let Some(v) = payload.max_amount {
            current.max_amount = v;
        }
        if let Some(v) = payload.enabled {
            current.enabled = v;
        }
        if let Some(v) = payload.expires_at {
            current.expires_at = v.and_then(|s| parse_beijing_string(&s).ok());
        }

        self.client
            .execute(
                "UPDATE admin_tokens SET name = $2, allowed_models = $3, max_tokens = $4, enabled = $5, expires_at = $6, max_amount = $7 WHERE token = $1",
                &[&token, &current.name, &join_allowed_models(&current.allowed_models), &current.max_tokens, &current.enabled, &current.expires_at.as_ref().map(to_beijing_string), &current.max_amount],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(Some(current))
    }

    async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute(
                "UPDATE admin_tokens SET enabled = $2 WHERE token = $1",
                &[&token, &enabled],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn get_token(&self, token: &str) -> Result<Option<AdminToken>, GatewayError> {
        let row = self.client
            .query_opt(
                "SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE token = $1",
                &[&token],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        if let Some(r) = row {
            Ok(Some(row_to_admin_token(&r)))
        } else {
            Ok(None)
        }
    }

    async fn get_token_by_id(&self, id: &str) -> Result<Option<AdminToken>, GatewayError> {
        let row = self.client
            .query_opt(
                "SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(row.map(|r| row_to_admin_token(&r)))
    }

    async fn list_tokens(&self) -> Result<Vec<AdminToken>, GatewayError> {
        let rows = self.client
            .query(
                "SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens ORDER BY created_at DESC",
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(rows.into_iter().map(|r| row_to_admin_token(&r)).collect())
    }

    async fn delete_token(&self, token: &str) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute("DELETE FROM admin_tokens WHERE token = $1", &[&token])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn delete_token_by_id(&self, id: &str) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute("DELETE FROM admin_tokens WHERE id = $1", &[&id])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn update_token_by_id(
        &self,
        id: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError> {
        let row = self
            .client
            .query_opt(
                "SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(r) = row else { return Ok(None) };
        let token: String = r.get(2);
        self.update_token(&token, payload).await
    }

    async fn set_enabled_by_id(&self, id: &str, enabled: bool) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute(
                "UPDATE admin_tokens SET enabled = $2 WHERE id = $1",
                &[&id, &enabled],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), GatewayError> {
        self.client
            .execute(
                "UPDATE admin_tokens SET amount_spent = COALESCE(amount_spent, 0) + $2 WHERE token = $1",
                &[&token, &delta],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }

    async fn add_usage_spent(
        &self,
        token: &str,
        prompt: i64,
        completion: i64,
        total: i64,
    ) -> Result<(), GatewayError> {
        self.client
            .execute(
                "UPDATE admin_tokens SET prompt_tokens_spent = COALESCE(prompt_tokens_spent,0) + $2, completion_tokens_spent = COALESCE(completion_tokens_spent,0) + $3, total_tokens_spent = COALESCE(total_tokens_spent,0) + $4 WHERE token = $1",
                &[&token, &prompt, &completion, &total],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }
}
