use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use tokio_postgres::{Client, NoTls};

use crate::error::GatewayError;
use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::logging::{CachedModel, RequestLog};
use crate::providers::openai::Model;
use crate::server::storage_traits::{BoxFuture, ModelCache, RequestLogStore, ProviderStore};
use crate::config::settings::{Provider, ProviderType, KeyLogStrategy};
use crate::logging::types::ProviderOpLog;

fn pg_err<E: std::fmt::Display>(e: E) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR), Some(format!("{}", e)))
}

struct PgPool {
    clients: Vec<Arc<Client>>,
    next: AtomicUsize,
}

impl PgPool {
    async fn connect_many(pg_url: &str, schema: &Option<String>, size: usize) -> Result<Self, GatewayError> {
        let mut clients = Vec::with_capacity(size.max(1));
        for _ in 0..size.max(1) {
            let (client, connection) = tokio_postgres::connect(pg_url, NoTls)
                .await
                .map_err(|e| GatewayError::Config(format!("Failed to connect postgres: {}", e)))?;
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
            let client = Arc::new(client);
            {
                let c = Arc::clone(&client);
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
                    loop {
                        interval.tick().await;
                        let _ = c.execute("SELECT 1", &[]).await;
                    }
                });
            }
            clients.push(client);
        }
        Ok(Self { clients, next: AtomicUsize::new(0) })
    }

    fn pick(&self) -> Arc<Client> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.clients.len().max(1);
        Arc::clone(&self.clients[idx])
    }
}

#[derive(Clone)]
pub struct PgLogStore {
    pool: Arc<PgPool>,
}

impl PgLogStore {
    pub async fn connect(pg_url: &str, schema: &Option<String>, pool_size: usize) -> Result<Self, GatewayError> {
        let pool = PgPool::connect_many(pg_url, schema, pool_size).await?;
        let store = Self { pool: Arc::new(pool) };
        // init tables
        let client = store.pool.pick();
        client.execute(
            r#"CREATE TABLE IF NOT EXISTS request_logs (
                id SERIAL PRIMARY KEY,
                timestamp TEXT NOT NULL,
                method TEXT NOT NULL,
                path TEXT NOT NULL,
                request_type TEXT NOT NULL,
                model TEXT,
                provider TEXT,
                api_key TEXT,
                status_code INTEGER NOT NULL,
                response_time_ms BIGINT NOT NULL,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                total_tokens INTEGER,
                cached_tokens INTEGER,
                reasoning_tokens INTEGER,
                error_message TEXT,
                client_token TEXT,
                amount_spent DOUBLE PRECISION
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init request_logs: {}", e)))?;
        // best-effort migration for existing deployments
        let _ = client.execute("ALTER TABLE request_logs ADD COLUMN amount_spent DOUBLE PRECISION", &[]).await;

        client.execute(
            r#"CREATE TABLE IF NOT EXISTS cached_models (
                id TEXT NOT NULL,
                provider TEXT NOT NULL,
                object TEXT NOT NULL,
                created BIGINT NOT NULL,
                owned_by TEXT NOT NULL,
                cached_at TEXT NOT NULL,
                PRIMARY KEY (id, provider)
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init cached_models: {}", e)))?;

        client.execute(
            r#"CREATE TABLE IF NOT EXISTS provider_ops_logs (
                id SERIAL PRIMARY KEY,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                provider TEXT,
                details TEXT
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init provider_ops_logs: {}", e)))?;

        client.execute(
            r#"CREATE TABLE IF NOT EXISTS model_prices (
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_price_per_million DOUBLE PRECISION NOT NULL,
                completion_price_per_million DOUBLE PRECISION NOT NULL,
                currency TEXT,
                PRIMARY KEY (provider, model)
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init model_prices: {}", e)))?;

        // Providers & provider_keys tables
        client.execute(
            r#"CREATE TABLE IF NOT EXISTS providers (
                name TEXT PRIMARY KEY,
                api_type TEXT NOT NULL,
                base_url TEXT NOT NULL,
                models_endpoint TEXT
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init providers: {}", e)))?;
        client.execute(
            r#"CREATE TABLE IF NOT EXISTS provider_keys (
                provider TEXT NOT NULL,
                key_value TEXT NOT NULL,
                enc BOOLEAN NOT NULL DEFAULT FALSE,
                active BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TEXT NOT NULL,
                PRIMARY KEY (provider, key_value)
            )"#,
            &[],
        ).await.map_err(|e| GatewayError::Config(format!("Failed to init provider_keys: {}", e)))?;

        Ok(store)
    }
}

impl RequestLogStore for PgLogStore {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO request_logs (timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent)
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
                    &[&to_beijing_string(&log.timestamp), &log.method, &log.path, &log.request_type, &log.model, &log.provider, &log.api_key, &i32::from(log.status_code), &log.response_time_ms, &log.prompt_tokens.map(|v| v as i32), &log.completion_tokens.map(|v| v as i32), &log.total_tokens.map(|v| v as i32), &log.cached_tokens.map(|v| v as i32), &log.reasoning_tokens.map(|v| v as i32), &log.error_message, &log.client_token, &log.amount_spent],
                )
                .await
                .map_err(pg_err)?;
            Ok(res as i64)
        })
    }

    fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = client
                .query(
                    "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs ORDER BY id DESC LIMIT $1",
                    &[&lim],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(RequestLog {
                    id: Some(r.get::<usize, i32>(0) as i64),
                    timestamp: parse_beijing_string(&r.get::<usize, String>(1)).unwrap_or(Utc::now()),
                    method: r.get(2),
                    path: r.get(3),
                    request_type: r.get(4),
                    model: r.get(5),
                    provider: r.get(6),
                    api_key: r.get(7),
                    status_code: r.get::<usize, i32>(8) as u16,
                    response_time_ms: r.get(9),
                    prompt_tokens: r.get::<usize, Option<i32>>(10).map(|v| v as u32),
                    completion_tokens: r.get::<usize, Option<i32>>(11).map(|v| v as u32),
                    total_tokens: r.get::<usize, Option<i32>>(12).map(|v| v as u32),
                    cached_tokens: r.get::<usize, Option<i32>>(13).map(|v| v as u32),
                    reasoning_tokens: r.get::<usize, Option<i32>>(14).map(|v| v as u32),
                    error_message: r.get(15),
                    client_token: r.get(16),
                    amount_spent: r.get(17),
                });
            }
            Ok(out)
        })
    }

    fn sum_total_tokens_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<u64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_one("SELECT COALESCE(SUM(total_tokens), 0) FROM request_logs WHERE client_token = $1", &[&token])
                .await
                .map_err(pg_err)?;
            let sum: i64 = row.get(0);
            Ok(sum as u64)
        })
    }

    fn get_logs_by_client_token<'a>(&'a self, token: &'a str, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let lim: i64 = limit as i64;
            let rows = client
                .query(
                    "SELECT id, timestamp, method, path, request_type, model, provider, api_key, status_code, response_time_ms, prompt_tokens, completion_tokens, total_tokens, cached_tokens, reasoning_tokens, error_message, client_token, amount_spent FROM request_logs WHERE client_token = $1 ORDER BY id DESC LIMIT $2",
                    &[&token, &lim],
                )
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(RequestLog {
                    id: Some(r.get::<usize, i32>(0) as i64),
                    timestamp: parse_beijing_string(&r.get::<usize, String>(1)).unwrap_or(Utc::now()),
                    method: r.get(2),
                    path: r.get(3),
                    request_type: r.get(4),
                    model: r.get(5),
                    provider: r.get(6),
                    api_key: r.get(7),
                    status_code: r.get::<usize, i32>(8) as u16,
                    response_time_ms: r.get(9),
                    prompt_tokens: r.get::<usize, Option<i32>>(10).map(|v| v as u32),
                    completion_tokens: r.get::<usize, Option<i32>>(11).map(|v| v as u32),
                    total_tokens: r.get::<usize, Option<i32>>(12).map(|v| v as u32),
                    cached_tokens: r.get::<usize, Option<i32>>(13).map(|v| v as u32),
                    reasoning_tokens: r.get::<usize, Option<i32>>(14).map(|v| v as u32),
                    error_message: r.get(15),
                    client_token: r.get(16),
                    amount_spent: r.get(17),
                });
            }
            Ok(out)
        })
    }

    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO provider_ops_logs (timestamp, operation, provider, details) VALUES ($1,$2,$3,$4)",
                    &[&to_beijing_string(&op.timestamp), &op.operation, &op.provider, &op.details],
                )
                .await
                .map_err(pg_err)?;
            Ok(res as i64)
        })
    }

    fn upsert_model_price<'a>(&'a self, provider: &'a str, model: &'a str, prompt_price_per_million: f64, completion_price_per_million: f64, currency: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            // 尝试 UPDATE，若未影响行则 INSERT（兼容不支持 ON CONFLICT 的库）
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE model_prices SET prompt_price_per_million=$3, completion_price_per_million=$4, currency=$5 WHERE provider=$1 AND model=$2",
                    &[&provider, &model, &prompt_price_per_million, &completion_price_per_million, &currency],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO model_prices (provider, model, prompt_price_per_million, completion_price_per_million, currency) VALUES ($1,$2,$3,$4,$5)",
                        &[&provider, &model, &prompt_price_per_million, &completion_price_per_million, &currency],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn get_model_price<'a>(&'a self, provider: &'a str, model: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt(
                    "SELECT prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = $1 AND model = $2",
                    &[&provider, &model],
                )
                .await
                .map_err(pg_err)?;
            Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
        })
    }

    fn list_model_prices<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>> {
        Box::pin(async move {
            let mut out = Vec::new();
            if let Some(p) = provider {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = $1 ORDER BY model",
                        &[&p],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows { out.push((r.get(0), r.get(1), r.get(2), r.get(3), r.get(4))); }
            } else {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices ORDER BY provider, model",
                        &[],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows { out.push((r.get(0), r.get(1), r.get(2), r.get(3), r.get(4))); }
            }
            Ok(out)
        })
    }

    fn sum_spent_amount_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<f64>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_one(
                    "SELECT COALESCE(SUM(COALESCE(prompt_tokens,0) * COALESCE(pp.prompt_price_per_million,0) / 1000000.0 + COALESCE(completion_tokens,0) * COALESCE(pp.completion_price_per_million,0) / 1000000.0), 0.0)
                     FROM request_logs rl JOIN model_prices pp ON rl.provider = pp.provider AND rl.model = pp.model WHERE rl.client_token = $1",
                    &[&token],
                )
                .await
                .map_err(pg_err)?;
            let sum: f64 = row.get(0);
            Ok(sum)
        })
    }
}

impl ModelCache for PgLogStore {
    fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = Utc::now();
            let client = self.pool.pick();
            client
                .execute("DELETE FROM cached_models WHERE provider = $1", &[&provider])
                .await
                .map_err(pg_err)?;
            for m in models {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at) VALUES ($1,$2,$3,$4,$5,$6)",
                        &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
        Box::pin(async move {
            let mut out = Vec::new();
            if let Some(p) = provider {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT id, provider, object, created, owned_by, cached_at FROM cached_models WHERE provider = $1 ORDER BY id",
                        &[&p],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push(CachedModel {
                        id: r.get(0),
                        provider: r.get(1),
                        object: r.get(2),
                        created: {
                            let v: i64 = r.get(3);
                            v as u64
                        },
                        owned_by: r.get(4),
                        cached_at: parse_beijing_string(&r.get::<usize, String>(5)).unwrap_or(Utc::now()),
                    });
                }
            } else {
                let client = self.pool.pick();
                let rows = client
                    .query(
                        "SELECT id, provider, object, created, owned_by, cached_at FROM cached_models ORDER BY provider, id",
                        &[],
                    )
                    .await
                    .map_err(pg_err)?;
                for r in rows {
                    out.push(CachedModel {
                        id: r.get(0),
                        provider: r.get(1),
                        object: r.get(2),
                        created: {
                            let v: i64 = r.get(3);
                            v as u64
                        },
                        owned_by: r.get(4),
                        cached_at: parse_beijing_string(&r.get::<usize, String>(5)).unwrap_or(Utc::now()),
                    });
                }
            }
            Ok(out)
        })
    }

    fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = Utc::now();
            for m in models {
                // 尝试 UPDATE，若未影响行则 INSERT
                let client = self.pool.pick();
                let affected = client
                    .execute(
                        "UPDATE cached_models SET object=$3, created=$4, owned_by=$5, cached_at=$6 WHERE id=$1 AND provider=$2",
                        &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                    )
                    .await
                    .map_err(pg_err)?;
                if affected == 0 {
                    let client = self.pool.pick();
                    client
                        .execute(
                            "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at) VALUES ($1,$2,$3,$4,$5,$6)",
                            &[&m.id, &provider, &m.object, &((m.created) as i64), &m.owned_by, &to_beijing_string(&now)],
                        )
                        .await
                        .map_err(pg_err)?;
                }
            }
            Ok(())
        })
    }

    fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            for id in ids {
                let client = self.pool.pick();
                client
                    .execute("DELETE FROM cached_models WHERE provider = $1 AND id = $2", &[&provider, id])
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }
}

impl ProviderStore for PgLogStore {
    fn insert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let res = client
                .execute(
                    "INSERT INTO providers (name, api_type, base_url, models_endpoint) VALUES ($1,$2,$3,$4)",
                    &[&provider.name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                )
                .await
                .map_err(pg_err)?;
            Ok(res > 0)
        })
    }

    fn upsert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE providers SET api_type=$2, base_url=$3, models_endpoint=$4 WHERE name=$1",
                    &[&provider.name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO providers (name, api_type, base_url, models_endpoint) VALUES ($1,$2,$3,$4)",
                        &[&provider.name, &provider_type_to_str(&provider.api_type), &provider.base_url, &provider.models_endpoint],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt("SELECT 1 FROM providers WHERE name = $1 LIMIT 1", &[&name])
                .await
                .map_err(pg_err)?;
            Ok(row.is_some())
        })
    }

    fn get_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let row = client
                .query_opt("SELECT name, api_type, base_url, models_endpoint FROM providers WHERE name = $1", &[&name])
                .await
                .map_err(pg_err)?;
            Ok(row.map(|r| Provider {
                name: r.get(0),
                api_type: provider_type_from_str(&r.get::<usize, String>(1)),
                base_url: r.get(2),
                api_keys: Vec::new(),
                models_endpoint: r.get(3),
            }))
        })
    }

    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query("SELECT name, api_type, base_url, models_endpoint FROM providers ORDER BY name", &[])
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(Provider {
                    name: r.get(0),
                    api_type: provider_type_from_str(&r.get::<usize, String>(1)),
                    base_url: r.get(2),
                    api_keys: Vec::new(),
                    models_endpoint: r.get(3),
                });
            }
            Ok(out)
        })
    }

    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            // cascade-like cleanup
            let client = self.pool.pick();
            client.execute("DELETE FROM provider_keys WHERE provider = $1", &[&name]).await.map_err(pg_err)?;
            let client = self.pool.pick();
            client.execute("DELETE FROM cached_models WHERE provider = $1", &[&name]).await.map_err(pg_err)?;
            let client = self.pool.pick();
            let res = client.execute("DELETE FROM providers WHERE name = $1", &[&name]).await.map_err(pg_err)?;
            Ok(res > 0)
        })
    }

    fn get_provider_keys<'a>(&'a self, provider: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move {
            let client = self.pool.pick();
            let rows = client
                .query("SELECT key_value, enc FROM provider_keys WHERE provider = $1 AND active = TRUE ORDER BY created_at", &[&provider])
                .await
                .map_err(pg_err)?;
            let mut out = Vec::new();
            for r in rows {
                let value: String = r.get(0);
                let enc: Option<bool> = r.get(1);
                let decrypted = match crate::crypto::unprotect(strategy, provider, &value, enc.unwrap_or(false)) {
                    Ok(v) => v,
                    Err(_) => String::new(),
                };
                if !decrypted.is_empty() { out.push(decrypted); }
            }
            Ok(out)
        })
    }

    fn add_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            let now = to_beijing_string(&Utc::now());
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let updated = client
                .execute(
                    "UPDATE provider_keys SET enc=$3, active=TRUE, created_at=$4 WHERE provider=$1 AND key_value=$2",
                    &[&provider, &stored, &enc, &now],
                )
                .await
                .map_err(pg_err)?;
            if updated == 0 {
                let client = self.pool.pick();
                client
                    .execute(
                        "INSERT INTO provider_keys (provider, key_value, enc, active, created_at) VALUES ($1,$2,$3,TRUE,$4)",
                        &[&provider, &stored, &enc, &now],
                    )
                    .await
                    .map_err(pg_err)?;
            }
            Ok(())
        })
    }

    fn remove_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            let (stored, enc) = crate::crypto::protect(strategy, provider, key);
            let client = self.pool.pick();
            let mut affected = client
                .execute("DELETE FROM provider_keys WHERE provider = $1 AND key_value = $2", &[&provider, &stored])
                .await
                .map_err(pg_err)?;
            if enc {
                let client = self.pool.pick();
                affected += client
                    .execute("DELETE FROM provider_keys WHERE provider = $1 AND key_value = $2", &[&provider, &key])
                    .await
                    .map_err(pg_err)?;
            }
            Ok(affected > 0)
        })
    }
}

fn provider_type_from_str(s: &str) -> ProviderType {
    match s.to_ascii_lowercase().as_str() {
        "openai" => ProviderType::OpenAI,
        "anthropic" => ProviderType::Anthropic,
        "zhipu" => ProviderType::Zhipu,
        _ => ProviderType::OpenAI,
    }
}

fn provider_type_to_str(t: &ProviderType) -> &'static str {
    match t {
        ProviderType::OpenAI => "openai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Zhipu => "zhipu",
    }
}
