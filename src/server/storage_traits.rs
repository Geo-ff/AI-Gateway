use std::future::Future;
use std::pin::Pin;

use crate::config::settings::{KeyLogStrategy, Provider};
use crate::logging::types::ProviderOpLog;
use crate::logging::{CachedModel, DatabaseLogger, ProviderKeyStatsAgg, RequestLog};
use crate::providers::openai::Model;
use crate::routing::{KeyRotationStrategy, ProviderKeyEntry};
use chrono::{DateTime, Utc};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type DateRangeFuture<'a> = BoxFuture<'a, rusqlite::Result<Option<(DateTime<Utc>, DateTime<Utc>)>>>;
type ModelPriceFuture<'a> = BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>>;
type ModelPriceListFuture<'a> =
    BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoriteKind {
    ClientToken,
    Provider,
}

impl FavoriteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FavoriteKind::ClientToken => "client_token",
            FavoriteKind::Provider => "provider",
        }
    }
}

pub trait FavoritesStore: Send + Sync {
    fn set_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
        favorite: bool,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;

    fn is_favorite<'a>(
        &'a self,
        kind: FavoriteKind,
        target: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn list_favorites<'a>(
        &'a self,
        kind: FavoriteKind,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>>;
}

// 日志存储抽象（可由 SQLite、Postgres 等实现）
pub trait RequestLogStore: Send + Sync {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>>;
    fn get_recent_logs<'a>(
        &'a self,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    fn get_recent_logs_with_cursor<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    #[allow(dead_code)]
    fn get_request_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    fn get_logs_by_method_path<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    #[allow(dead_code)]
    fn sum_total_tokens_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<u64>>;
    fn get_logs_by_client_token<'a>(
        &'a self,
        token: &'a str,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>>;
    fn count_requests_by_client_token<'a>(
        &'a self,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, i64)>>>;
    fn get_request_log_date_range<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
    ) -> DateRangeFuture<'a>;
    fn aggregate_provider_key_stats<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        provider: &'a str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyStatsAgg>>>;
    // provider ops audit log
    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>>;
    fn get_provider_ops_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderOpLog>>>;
    // pricing & billing
    fn upsert_model_price<'a>(
        &'a self,
        provider: &'a str,
        model: &'a str,
        prompt_price_per_million: f64,
        completion_price_per_million: f64,
        currency: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_model_price<'a>(&'a self, provider: &'a str, model: &'a str) -> ModelPriceFuture<'a>;
    fn list_model_prices<'a>(&'a self, provider: Option<&'a str>) -> ModelPriceListFuture<'a>;
    fn sum_spent_amount_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<f64>>;
}

// 模型缓存抽象（可由 SQLite、Redis 等实现）
pub trait ModelCache: Send + Sync {
    fn cache_models<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_cached_models<'a>(
        &'a self,
        provider: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>>;
    fn cache_models_append<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn remove_cached_models<'a>(
        &'a self,
        provider: &'a str,
        ids: &'a [String],
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
}

// 供应商与密钥的存储抽象（SQLite / Postgres 实现）
pub trait ProviderStore: Send + Sync {
    fn insert_provider<'a>(
        &'a self,
        provider: &'a Provider,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn upsert_provider<'a>(&'a self, provider: &'a Provider)
    -> BoxFuture<'a, rusqlite::Result<()>>;
    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn get_provider<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>>;
    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>>;
    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>>;
    fn set_provider_enabled<'a>(
        &'a self,
        provider: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn get_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<KeyRotationStrategy>>;

    fn set_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
        strategy: KeyRotationStrategy,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn get_provider_keys<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>>;
    fn add_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn remove_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn list_provider_keys_raw<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntry>>>;

    fn list_provider_keys_raw_with_created_at<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntryWithCreatedAt>>>;

    fn set_provider_key_weight<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        weight: u32,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn set_provider_key_active<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        active: bool,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn list_providers_with_keys<'a>(
        &'a self,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move {
            let mut out = self.list_providers().await?;
            for p in &mut out {
                p.api_keys = self.get_provider_keys(&p.name, strategy).await?;
            }
            Ok(out)
        })
    }

    // provider-scoped model redirects
    fn list_model_redirects<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String)>>>;
    fn replace_model_redirects<'a>(
        &'a self,
        provider: &'a str,
        redirects: &'a [(String, String)],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn delete_model_redirect<'a>(
        &'a self,
        provider: &'a str,
        source_model: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;
}

#[derive(Debug, Clone)]
pub struct AdminPublicKeyRecord {
    pub fingerprint: String,
    pub public_key: Vec<u8>,
    pub comment: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ProviderKeyEntryWithCreatedAt {
    pub value: String,
    pub active: bool,
    pub weight: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TuiSessionRecord {
    pub session_id: String,
    pub fingerprint: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub last_code_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct LoginCodeRecord {
    pub code_hash: String,
    pub session_id: String,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub uses: u32,
    pub disabled: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebSessionRecord {
    pub session_id: String,
    pub fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub issued_by_code: Option<String>,
}

pub trait LoginStore: Send + Sync {
    fn insert_admin_key<'a>(
        &'a self,
        key: &'a AdminPublicKeyRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<AdminPublicKeyRecord>>>;
    fn touch_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
        when: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn list_admin_keys<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<AdminPublicKeyRecord>>>;
    fn delete_admin_key<'a>(
        &'a self,
        fingerprint: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn create_tui_session<'a>(
        &'a self,
        session: &'a TuiSessionRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<TuiSessionRecord>>>;
    fn list_tui_sessions<'a>(
        &'a self,
        fingerprint: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<TuiSessionRecord>>>;
    fn update_tui_session_last_code<'a>(
        &'a self,
        session_id: &'a str,
        when: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn revoke_tui_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;

    fn disable_codes_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn insert_login_code<'a>(
        &'a self,
        code: &'a LoginCodeRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn redeem_login_code<'a>(
        &'a self,
        code_hash: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>>;
    fn get_latest_login_code_for_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<LoginCodeRecord>>>;

    fn insert_web_session<'a>(
        &'a self,
        session: &'a WebSessionRecord,
    ) -> BoxFuture<'a, rusqlite::Result<()>>;
    fn get_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<WebSessionRecord>>>;
    fn revoke_web_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>>;
}

// 现有的 DatabaseLogger 作为两种接口的默认实现
impl RequestLogStore for DatabaseLogger {
    fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move { self.log_request(log).await })
    }

    fn get_recent_logs<'a>(
        &'a self,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_recent_logs(limit).await })
    }

    fn get_recent_logs_with_cursor<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_recent_logs_with_cursor(limit, cursor).await })
    }

    fn get_request_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_request_logs(limit, cursor).await })
    }

    fn get_logs_by_method_path<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move {
            self.get_logs_by_method_path(method, path, limit, cursor)
                .await
        })
    }

    fn sum_total_tokens_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<u64>> {
        Box::pin(async move { self.sum_total_tokens_by_client_token(token).await })
    }

    fn get_logs_by_client_token<'a>(
        &'a self,
        token: &'a str,
        limit: i32,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
        Box::pin(async move { self.get_logs_by_client_token(token, limit).await })
    }

    fn count_requests_by_client_token<'a>(
        &'a self,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, i64)>>> {
        Box::pin(async move { self.count_requests_by_client_token().await })
    }

    fn get_request_log_date_range<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<(DateTime<Utc>, DateTime<Utc>)>>> {
        Box::pin(async move { self.request_log_date_range(method, path).await })
    }

    fn aggregate_provider_key_stats<'a>(
        &'a self,
        method: &'a str,
        path: &'a str,
        provider: &'a str,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyStatsAgg>>> {
        Box::pin(async move {
            self.aggregate_provider_key_stats(method, path, provider, since, until)
                .await
        })
    }

    fn log_provider_op<'a>(&'a self, op: ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
        Box::pin(async move { self.log_provider_op(op).await })
    }

    fn get_provider_ops_logs<'a>(
        &'a self,
        limit: i32,
        cursor: Option<i64>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderOpLog>>> {
        Box::pin(async move { self.get_provider_ops_logs(limit, cursor).await })
    }

    fn upsert_model_price<'a>(
        &'a self,
        provider: &'a str,
        model: &'a str,
        prompt_price_per_million: f64,
        completion_price_per_million: f64,
        currency: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move {
            self.upsert_model_price(
                provider,
                model,
                prompt_price_per_million,
                completion_price_per_million,
                currency,
            )
            .await
        })
    }

    fn get_model_price<'a>(
        &'a self,
        provider: &'a str,
        model: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>> {
        Box::pin(async move { self.get_model_price(provider, model).await })
    }

    fn list_model_prices<'a>(
        &'a self,
        provider: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>> {
        Box::pin(async move { self.list_model_prices(provider).await })
    }

    fn sum_spent_amount_by_client_token<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<f64>> {
        Box::pin(async move { self.sum_spent_amount_by_client_token(token).await })
    }
}

impl ModelCache for DatabaseLogger {
    fn cache_models<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models(provider, models).await })
    }

    fn get_cached_models<'a>(
        &'a self,
        provider: Option<&'a str>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
        Box::pin(async move { self.get_cached_models(provider).await })
    }

    fn cache_models_append<'a>(
        &'a self,
        provider: &'a str,
        models: &'a [Model],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.cache_models_append(provider, models).await })
    }

    fn remove_cached_models<'a>(
        &'a self,
        provider: &'a str,
        ids: &'a [String],
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.remove_cached_models(provider, ids).await })
    }
}

impl ProviderStore for DatabaseLogger {
    fn insert_provider<'a>(
        &'a self,
        provider: &'a Provider,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.insert_provider(provider).await })
    }
    fn upsert_provider<'a>(
        &'a self,
        provider: &'a Provider,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.upsert_provider(provider).await })
    }
    fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.provider_exists(name).await })
    }
    fn get_provider<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>> {
        Box::pin(async move { self.get_provider(name).await })
    }
    fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
        Box::pin(async move { self.list_providers().await })
    }
    fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.delete_provider(name).await })
    }
    fn set_provider_enabled<'a>(
        &'a self,
        provider: &'a str,
        enabled: bool,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.set_provider_enabled(provider, enabled).await })
    }

    fn get_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<KeyRotationStrategy>> {
        Box::pin(async move { self.get_provider_key_rotation_strategy(provider).await })
    }

    fn set_provider_key_rotation_strategy<'a>(
        &'a self,
        provider: &'a str,
        strategy: KeyRotationStrategy,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            self.set_provider_key_rotation_strategy(provider, strategy)
                .await
        })
    }
    fn get_provider_keys<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
        Box::pin(async move { self.get_provider_keys(provider, strategy).await })
    }
    fn add_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.add_provider_key(provider, key, strategy).await })
    }
    fn remove_provider_key<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.remove_provider_key(provider, key, strategy).await })
    }

    fn list_provider_keys_raw<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntry>>> {
        Box::pin(async move { self.list_provider_keys_raw(provider, strategy).await })
    }

    fn list_provider_keys_raw_with_created_at<'a>(
        &'a self,
        provider: &'a str,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<ProviderKeyEntryWithCreatedAt>>> {
        Box::pin(async move {
            self.list_provider_keys_raw_with_created_at(provider, strategy)
                .await
        })
    }

    fn set_provider_key_weight<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        weight: u32,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            self.set_provider_key_weight(provider, key, weight, strategy)
                .await
        })
    }

    fn set_provider_key_active<'a>(
        &'a self,
        provider: &'a str,
        key: &'a str,
        active: bool,
        strategy: &'a Option<KeyLogStrategy>,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move {
            self.set_provider_key_active(provider, key, active, strategy)
                .await
        })
    }

    fn list_model_redirects<'a>(
        &'a self,
        provider: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String)>>> {
        Box::pin(async move { self.list_model_redirects(provider).await })
    }

    fn replace_model_redirects<'a>(
        &'a self,
        provider: &'a str,
        redirects: &'a [(String, String)],
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, rusqlite::Result<()>> {
        Box::pin(async move { self.replace_model_redirects(provider, redirects, now).await })
    }

    fn delete_model_redirect<'a>(
        &'a self,
        provider: &'a str,
        source_model: &'a str,
    ) -> BoxFuture<'a, rusqlite::Result<bool>> {
        Box::pin(async move { self.delete_model_redirect(provider, source_model).await })
    }
}
