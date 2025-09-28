use std::future::Future;
use std::pin::Pin;

use crate::logging::{CachedModel, DatabaseLogger, RequestLog};
use crate::logging::types::ProviderOpLog;
use crate::providers::openai::Model;
use crate::config::settings::{Provider, KeyLogStrategy};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// 日志存储抽象（可由 SQLite、Postgres 等实现）
pub trait RequestLogStore: Send + Sync {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>>;
    fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    fn sum_total_tokens_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<u64>>;
    fn get_logs_by_client_token<'a>(&'a self, token: &'a str, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    // provider ops audit log
    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>>;
    // pricing & billing
    fn upsert_model_price<'a>(&'a self, provider: &'a str, model: &'a str, prompt_price_per_million: f64, completion_price_per_million: f64, currency: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_model_price<'a>(&'a self, provider: &'a str, model: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>>;
    fn list_model_prices<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>>;
    fn sum_spent_amount_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<f64>>;
}

// 模型缓存抽象（可由 SQLite、Redis 等实现）
pub trait ModelCache: Send + Sync {
    fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>>;
    fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>>;
}

// 供应商与密钥的存储抽象（SQLite / Postgres 实现）
pub trait ProviderStore: Send + Sync {
    fn insert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn upsert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn get_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>>;
    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>>;
    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn get_provider_keys<'a>(&'a self, provider: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<Vec<String>>>;
    fn add_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn remove_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn list_providers_with_keys<'a>(&'a self, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move {
            let mut out = self.list_providers().await?;
            for p in &mut out {
                p.api_keys = self.get_provider_keys(&p.name, strategy).await?;
            }
            Ok(out)
        })
    }
}

// 现有的 DatabaseLogger 作为两种接口的默认实现
impl RequestLogStore for DatabaseLogger {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move { self.log_request(log).await })
    }

    fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_recent_logs(limit).await })
    }

    fn sum_total_tokens_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<u64>> {
        Box::pin(async move { self.sum_total_tokens_by_client_token(token).await })
    }

    fn get_logs_by_client_token<'a>(&'a self, token: &'a str, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_logs_by_client_token(token, limit).await })
    }

    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move { self.log_provider_op(op).await })
    }

    fn upsert_model_price<'a>(&'a self, provider: &'a str, model: &'a str, prompt_price_per_million: f64, completion_price_per_million: f64, currency: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.upsert_model_price(provider, model, prompt_price_per_million, completion_price_per_million, currency).await })
    }

    fn get_model_price<'a>(&'a self, provider: &'a str, model: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>> {
        Box::pin(async move { self.get_model_price(provider, model).await })
    }

    fn list_model_prices<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>> {
        Box::pin(async move { self.list_model_prices(provider).await })
    }

    fn sum_spent_amount_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<f64>> {
        Box::pin(async move { self.sum_spent_amount_by_client_token(token).await })
    }
}

impl ModelCache for DatabaseLogger {
    fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models(provider, models).await })
    }

    fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
        Box::pin(async move { self.get_cached_models(provider).await })
    }

    fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models_append(provider, models).await })
    }

    fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.remove_cached_models(provider, ids).await })
    }
}

impl ProviderStore for DatabaseLogger {
    fn insert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.insert_provider(provider).await })
    }
    fn upsert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.upsert_provider(provider).await })
    }
    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.provider_exists(name).await })
    }
    fn get_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>> {
        Box::pin(async move { self.get_provider(name).await })
    }
    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move { self.list_providers().await })
    }
    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.delete_provider(name).await })
    }
    fn get_provider_keys<'a>(&'a self, provider: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move { self.get_provider_keys(provider, strategy).await })
    }
    fn add_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.add_provider_key(provider, key, strategy).await })
    }
    fn remove_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, strategy: &'a Option<KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.remove_provider_key(provider, key, strategy).await })
    }
}
