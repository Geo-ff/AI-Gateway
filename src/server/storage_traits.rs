use std::future::Future;
use std::pin::Pin;

use crate::logging::{CachedModel, DatabaseLogger, RequestLog};
use crate::providers::openai::Model;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// 日志存储抽象（可由 SQLite、Postgres 等实现）
pub trait RequestLogStore: Send + Sync {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>>;
    fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
}

// 模型缓存抽象（可由 SQLite、Redis 等实现）
pub trait ModelCache: Send + Sync {
    fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>>;
    fn is_cache_fresh<'a>(&'a self, provider: &'a str, max_age_minutes: i64) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>>;
}

// 现有的 DatabaseLogger 作为两种接口的默认实现
impl RequestLogStore for DatabaseLogger {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move { self.log_request(log).await })
    }

    fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_recent_logs(limit).await })
    }
}

impl ModelCache for DatabaseLogger {
    fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models(provider, models).await })
    }

    fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
        Box::pin(async move { self.get_cached_models(provider).await })
    }

    fn is_cache_fresh<'a>(&'a self, provider: &'a str, max_age_minutes: i64) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.is_cache_fresh(provider, max_age_minutes).await })
    }

    fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [Model]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models_append(provider, models).await })
    }

    fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.remove_cached_models(provider, ids).await })
    }
}
