use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;
use crate::logging::time::{parse_beijing_string, to_beijing_string};

const CLIENT_TOKEN_ID_PREFIX: &str = "atk_";

pub(crate) fn client_token_id_for_token(token: &str) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    let hex = hex::encode(hasher.finalize());
    format!("{}{}", CLIENT_TOKEN_ID_PREFIX, &hex[..24])
}

pub(crate) fn normalize_client_token_name(name: Option<String>, id: &str) -> String {
    let trimmed = name.map(|v| v.trim().to_string());
    if let Some(v) = trimmed.filter(|v| !v.is_empty()) {
        return v;
    }
    // Default: stable, non-sensitive, human-friendly
    let suffix = id.strip_prefix(CLIENT_TOKEN_ID_PREFIX).unwrap_or(id);
    format!("token-{}", &suffix[..8.min(suffix.len())])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientToken {
    pub id: String,
    pub user_id: Option<String>,
    pub name: String,
    pub token: String,
    pub allowed_models: Option<Vec<String>>, // None 表示不限制
    pub max_tokens: Option<i64>,             // 兼容旧字段（不再使用）
    pub max_amount: Option<f64>,             // 金额额度（单位自定义，如 USD/CNY）
    pub enabled: bool,
    pub expires_at: Option<DateTime<Utc>>, // None 表示不过期
    pub created_at: DateTime<Utc>,
    pub amount_spent: f64,                 // 累计消费金额（默认 0）
    pub prompt_tokens_spent: i64,          // 累计提示/输入 tokens
    pub completion_tokens_spent: i64,      // 累计补全/回复 tokens
    pub total_tokens_spent: i64,           // 累计总 tokens
    pub remark: Option<String>,            // 备注
    pub organization_id: Option<String>,   // 所属组织 ID（暂按字符串）
    pub ip_whitelist: Option<Vec<String>>, // IP 白名单（JSON 数组）
    pub ip_blacklist: Option<Vec<String>>, // IP 黑名单（JSON 数组）
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTokenPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
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
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub ip_whitelist: Option<Vec<String>>,
    #[serde(default)]
    pub ip_blacklist: Option<Vec<String>>,
}

fn default_enabled_true() -> bool {
    true
}

fn deserialize_patch_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTokenPayload {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub allowed_models: Option<Option<Vec<String>>>, // None -> 不修改；Some(Some(vec)) -> 设置；Some(None) -> 清空
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub max_tokens: Option<Option<i64>>, // 兼容旧字段（忽略）
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub max_amount: Option<Option<f64>>, // None -> 不修改；Some(Some(v)) -> 设置；Some(None) -> 清空
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub expires_at: Option<Option<String>>, // None -> 不修改；Some(Some(s)) -> 设置；Some(None) -> 清空
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub remark: Option<Option<String>>, // None -> 不修改；Some(Some(s)) -> 设置；Some(None) -> 清空
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub organization_id: Option<Option<String>>, // 同上
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub ip_whitelist: Option<Option<Vec<String>>>, // 同上
    #[serde(default, deserialize_with = "deserialize_patch_option")]
    pub ip_blacklist: Option<Option<Vec<String>>>, // 同上
}

#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn create_token(&self, payload: CreateTokenPayload) -> Result<ClientToken, GatewayError>;
    async fn update_token(
        &self,
        token: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<ClientToken>, GatewayError>;
    async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, GatewayError>;
    async fn get_token(&self, token: &str) -> Result<Option<ClientToken>, GatewayError>;
    async fn get_token_by_id(&self, id: &str) -> Result<Option<ClientToken>, GatewayError>;
    async fn get_token_by_id_scoped(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<Option<ClientToken>, GatewayError>;
    async fn list_tokens(&self) -> Result<Vec<ClientToken>, GatewayError>;
    async fn list_tokens_by_user(&self, user_id: &str) -> Result<Vec<ClientToken>, GatewayError>;
    async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), GatewayError>;
    async fn add_usage_spent(
        &self,
        token: &str,
        prompt: i64,
        completion: i64,
        total: i64,
    ) -> Result<(), GatewayError>;
    #[allow(dead_code)]
    async fn delete_token(&self, token: &str) -> Result<bool, GatewayError>;
    async fn delete_token_by_id(&self, id: &str) -> Result<bool, GatewayError>;
    async fn update_token_by_id(
        &self,
        id: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<ClientToken>, GatewayError>;
    async fn set_enabled_by_id(&self, id: &str, enabled: bool) -> Result<bool, GatewayError>;
}

// SQLite 的实现由 DatabaseLogger 提供（见 logging/database_client_tokens.rs）

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

pub(crate) fn encode_json_string_list(
    field: &str,
    v: &Option<Vec<String>>,
) -> Result<Option<String>, GatewayError> {
    match v {
        None => Ok(None),
        Some(list) => serde_json::to_string(list)
            .map(Some)
            .map_err(|e| GatewayError::Config(format!("Failed to encode {}: {}", field, e))),
    }
}

pub(crate) fn decode_json_string_list(
    field: &str,
    s: Option<String>,
) -> Result<Option<Vec<String>>, GatewayError> {
    let Some(raw) = s else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str::<Vec<String>>(trimmed)
        .map(Some)
        .map_err(|e| {
            GatewayError::Config(format!(
                "DB decode error: client_tokens.{} invalid JSON: {}",
                field, e
            ))
        })
}

fn row_to_client_token(r: &tokio_postgres::Row) -> Result<ClientToken, GatewayError> {
    let id_opt: Option<String> = r.get(0);
    let user_id: Option<String> = r.get(1);
    let name_opt: Option<String> = r.get(2);
    let token: String = r.get(3);
    let allowed_s: Option<String> = r.get(4);
    let max_tokens: Option<i64> = r.get(5);
    let enabled: bool = r.get::<usize, Option<bool>>(6).unwrap_or(true);
    let expires_s: Option<String> = r.get(7);
    let created_s: String = r.get(8);
    let max_amount: Option<f64> = r.get(9);
    let amount_spent: f64 = r.get::<usize, Option<f64>>(10).unwrap_or(0.0);
    let prompt_tokens_spent: i64 = r.get::<usize, Option<i64>>(11).unwrap_or(0);
    let completion_tokens_spent: i64 = r.get::<usize, Option<i64>>(12).unwrap_or(0);
    let total_tokens_spent: i64 = r.get::<usize, Option<i64>>(13).unwrap_or(0);
    let remark: Option<String> = r.get(14);
    let organization_id: Option<String> = r.get(15);
    let ip_whitelist_s: Option<String> = r.get(16);
    let ip_blacklist_s: Option<String> = r.get(17);
    let id = id_opt.unwrap_or_else(|| client_token_id_for_token(&token));
    let name = normalize_client_token_name(name_opt, &id);
    Ok(ClientToken {
        id,
        user_id,
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
        remark,
        organization_id,
        ip_whitelist: decode_json_string_list("ip_whitelist", ip_whitelist_s)?,
        ip_blacklist: decode_json_string_list("ip_blacklist", ip_blacklist_s)?,
    })
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
        ensure_client_tokens_table_pg(&client).await?;
        let store = Self {
            client: std::sync::Arc::new(client),
        };
        // keepalive（带抖动），降低空闲回收的概率并避免集群齐刷刷触发
        crate::db::postgres::spawn_keepalive(std::sync::Arc::clone(&store.client), 240, 420);
        Ok(store)
    }
}

const CLIENT_TOKENS_TABLE: &str = "client_tokens";

fn quote_pg_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

async fn table_exists_pg(
    client: &tokio_postgres::Client,
    table_name: &str,
) -> Result<bool, GatewayError> {
    let row = client
        .query_opt(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = $1",
            &[&table_name],
        )
        .await
        .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
    Ok(row.is_some())
}

async fn find_legacy_tokens_table_pg(
    client: &tokio_postgres::Client,
) -> Result<Option<String>, GatewayError> {
    // Heuristic: locate a token table in current_schema by its core columns.
    let rows = client
        .query(
            "SELECT table_name
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND column_name IN ('token', 'enabled', 'created_at', 'allowed_models')
             GROUP BY table_name
             HAVING COUNT(DISTINCT column_name) = 4
             LIMIT 8",
            &[],
        )
        .await
        .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

    for r in rows {
        let name: String = r.get(0);
        if name == CLIENT_TOKENS_TABLE {
            continue;
        }
        return Ok(Some(name));
    }
    Ok(None)
}

async fn ensure_client_tokens_table_pg(
    client: &tokio_postgres::Client,
) -> Result<(), GatewayError> {
    let legacy = if !table_exists_pg(client, CLIENT_TOKENS_TABLE).await? {
        find_legacy_tokens_table_pg(client).await?
    } else {
        None
    };
    if let Some(legacy) = legacy {
        let sql = format!(
            "ALTER TABLE {} RENAME TO {}",
            quote_pg_ident(&legacy),
            CLIENT_TOKENS_TABLE
        );
        let _ = client.execute(&sql, &[]).await;
    }

    client
        .execute(
            r#"CREATE TABLE IF NOT EXISTS client_tokens (
                id TEXT UNIQUE,
                user_id TEXT,
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
                total_tokens_spent BIGINT DEFAULT 0,
                remark TEXT,
                organization_id TEXT,
                ip_whitelist TEXT,
                ip_blacklist TEXT
            )"#,
            &[],
        )
        .await
        .map_err(|e| GatewayError::Config(format!("Failed to init client_tokens: {}", e)))?;

    // Migration (best-effort)
    let _ = client
        .execute("ALTER TABLE client_tokens ADD COLUMN id TEXT", &[])
        .await;
    let _ = client
        .execute("ALTER TABLE client_tokens ADD COLUMN name TEXT", &[])
        .await;
    let _ = client
        .execute("ALTER TABLE client_tokens ADD COLUMN user_id TEXT", &[])
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN max_amount DOUBLE PRECISION",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN amount_spent DOUBLE PRECISION DEFAULT 0",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN prompt_tokens_spent BIGINT DEFAULT 0",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN completion_tokens_spent BIGINT DEFAULT 0",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN total_tokens_spent BIGINT DEFAULT 0",
            &[],
        )
        .await;
    let _ = client
        .execute("ALTER TABLE client_tokens ADD COLUMN remark TEXT", &[])
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN organization_id TEXT",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN ip_whitelist TEXT",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "ALTER TABLE client_tokens ADD COLUMN ip_blacklist TEXT",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS client_tokens_id_uidx ON client_tokens(id)",
            &[],
        )
        .await;
    let _ = client
        .execute(
            "CREATE INDEX IF NOT EXISTS client_tokens_user_id_idx ON client_tokens(user_id)",
            &[],
        )
        .await;

    // Backfill id/name for existing rows (best-effort)
    if let Ok(rows) = client
        .query(
            "SELECT token FROM client_tokens WHERE id IS NULL OR id = '' OR name IS NULL OR name = ''",
            &[],
        )
        .await
    {
        for r in rows {
            let tok: String = r.get(0);
            let id = client_token_id_for_token(&tok);
            let name = normalize_client_token_name(None, &id);
            let _ = client
                .execute(
                    "UPDATE client_tokens SET id = $2 WHERE token = $1 AND (id IS NULL OR id = '')",
                    &[&tok, &id],
                )
                .await;
            let _ = client
                .execute(
                    "UPDATE client_tokens SET name = $2 WHERE token = $1 AND (name IS NULL OR name = '')",
                    &[&tok, &name],
                )
                .await;
        }
    }

    Ok(())
}

#[async_trait]
impl TokenStore for PgTokenStore {
    async fn create_token(&self, payload: CreateTokenPayload) -> Result<ClientToken, GatewayError> {
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
        let id = client_token_id_for_token(&token);
        let name = normalize_client_token_name(payload.name.clone(), &id);
        let now = Utc::now();
        let allowed_models_s = join_allowed_models(&payload.allowed_models);
        let expires_s = payload.expires_at.clone();
        let ip_whitelist_s = encode_json_string_list("ip_whitelist", &payload.ip_whitelist)?;
        let ip_blacklist_s = encode_json_string_list("ip_blacklist", &payload.ip_blacklist)?;
        self.client
            .execute(
                "INSERT INTO client_tokens (id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, 0, 0, 0, $11, $12, $13, $14)",
                &[&id, &payload.user_id, &name, &token, &allowed_models_s, &payload.max_tokens, &payload.enabled, &expires_s, &to_beijing_string(&now), &payload.max_amount, &payload.remark, &payload.organization_id, &ip_whitelist_s, &ip_blacklist_s],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(ClientToken {
            id,
            user_id: payload.user_id,
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
            remark: payload.remark,
            organization_id: payload.organization_id,
            ip_whitelist: payload.ip_whitelist,
            ip_blacklist: payload.ip_blacklist,
        })
    }

    async fn update_token(
        &self,
        token: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<ClientToken>, GatewayError> {
        // read existing
        let row = self.client
            .query_opt(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE token = $1",
                &[&token],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(r) = row else { return Ok(None) };
        let mut current = row_to_client_token(&r)?;

        if let Some(v) = payload.name {
            current.name = normalize_client_token_name(Some(v), &current.id);
        }
        if let Some(v) = payload.allowed_models {
            current.allowed_models = v;
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
        if let Some(v) = payload.remark {
            current.remark = v;
        }
        if let Some(v) = payload.organization_id {
            current.organization_id = v;
        }
        if let Some(v) = payload.ip_whitelist {
            current.ip_whitelist = v;
        }
        if let Some(v) = payload.ip_blacklist {
            current.ip_blacklist = v;
        }

        let ip_whitelist_s = encode_json_string_list("ip_whitelist", &current.ip_whitelist)?;
        let ip_blacklist_s = encode_json_string_list("ip_blacklist", &current.ip_blacklist)?;
        self.client
            .execute(
                "UPDATE client_tokens SET name = $2, allowed_models = $3, max_tokens = $4, enabled = $5, expires_at = $6, max_amount = $7, remark = $8, organization_id = $9, ip_whitelist = $10, ip_blacklist = $11 WHERE token = $1",
                &[&token, &current.name, &join_allowed_models(&current.allowed_models), &current.max_tokens, &current.enabled, &current.expires_at.as_ref().map(to_beijing_string), &current.max_amount, &current.remark, &current.organization_id, &ip_whitelist_s, &ip_blacklist_s],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(Some(current))
    }

    async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute(
                "UPDATE client_tokens SET enabled = $2 WHERE token = $1",
                &[&token, &enabled],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn get_token(&self, token: &str) -> Result<Option<ClientToken>, GatewayError> {
        let row = self.client
            .query_opt(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE token = $1",
                &[&token],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        if let Some(r) = row {
            Ok(Some(row_to_client_token(&r)?))
        } else {
            Ok(None)
        }
    }

    async fn get_token_by_id(&self, id: &str) -> Result<Option<ClientToken>, GatewayError> {
        let row = self.client
            .query_opt(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        match row {
            Some(r) => Ok(Some(row_to_client_token(&r)?)),
            None => Ok(None),
        }
    }

    async fn get_token_by_id_scoped(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<Option<ClientToken>, GatewayError> {
        let row = self
            .client
            .query_opt(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE id = $1 AND user_id = $2",
                &[&id, &user_id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        match row {
            Some(r) => Ok(Some(row_to_client_token(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_tokens(&self) -> Result<Vec<ClientToken>, GatewayError> {
        let rows = self.client
            .query(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens ORDER BY created_at DESC",
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        rows.into_iter()
            .map(|r| row_to_client_token(&r))
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_tokens_by_user(&self, user_id: &str) -> Result<Vec<ClientToken>, GatewayError> {
        let rows = self
            .client
            .query(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE user_id = $1 ORDER BY created_at DESC",
                &[&user_id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        rows.into_iter()
            .map(|r| row_to_client_token(&r))
            .collect::<Result<Vec<_>, _>>()
    }

    async fn delete_token(&self, token: &str) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute("DELETE FROM client_tokens WHERE token = $1", &[&token])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn delete_token_by_id(&self, id: &str) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute("DELETE FROM client_tokens WHERE id = $1", &[&id])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn update_token_by_id(
        &self,
        id: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<ClientToken>, GatewayError> {
        let row = self
            .client
            .query_opt(
                "SELECT id, user_id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent, remark, organization_id, ip_whitelist, ip_blacklist FROM client_tokens WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(r) = row else { return Ok(None) };
        let token: String = r.get(3);
        self.update_token(&token, payload).await
    }

    async fn set_enabled_by_id(&self, id: &str, enabled: bool) -> Result<bool, GatewayError> {
        let res = self
            .client
            .execute(
                "UPDATE client_tokens SET enabled = $2 WHERE id = $1",
                &[&id, &enabled],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(res > 0)
    }

    async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), GatewayError> {
        self.client
            .execute(
                "UPDATE client_tokens SET amount_spent = COALESCE(amount_spent, 0) + $2 WHERE token = $1",
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
                "UPDATE client_tokens SET prompt_tokens_spent = COALESCE(prompt_tokens_spent,0) + $2, completion_tokens_spent = COALESCE(completion_tokens_spent,0) + $3, total_tokens_spent = COALESCE(total_tokens_spent,0) + $4 WHERE token = $1",
                &[&token, &prompt, &completion, &total],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(())
    }
}
