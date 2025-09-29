// use std::sync::Arc;
// use std::time::Duration;
// use std::{collections::HashMap};

// use axum::Router;
// use axum::http::{Request, StatusCode};
// use axum::body::Body;
// use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, black_box};
// use tower::ServiceExt; // for .oneshot

// use gateway::config::{Settings, LoadBalancing, BalanceStrategy, ServerConfig, LoggingConfig, Provider, ProviderType};
// use gateway::logging::{RequestLog, CachedModel};
// use gateway::server::handlers;
// use gateway::server::storage_traits::{RequestLogStore, ModelCache, ProviderStore, BoxFuture};
// use gateway::admin::{TokenStore, AdminToken, CreateTokenPayload, UpdateTokenPayload};
// use gateway::server::AppState;
// use gateway::providers::openai::Model as OpenAIModel;
// use chrono::{Utc};
// use async_trait::async_trait;

// // --------------------- In-memory stores for benchmarking ---------------------

// #[derive(Clone, Default)]
// struct MemStore {
//     logs: Arc<tokio::sync::RwLock<Vec<RequestLog>>>,
//     cached: Arc<tokio::sync::RwLock<Vec<CachedModel>>>,
//     providers: Arc<tokio::sync::RwLock<HashMap<String, Provider>>>,
//     provider_keys: Arc<tokio::sync::RwLock<HashMap<String, Vec<String>>>>,
//     prices: Arc<tokio::sync::RwLock<HashMap<(String, String), (f64, f64, Option<String>)>>>,
// }

// impl RequestLogStore for MemStore {
//     fn log_request<'a>(&'a self, log: RequestLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
//         Box::pin(async move {
//             let mut guard = self.logs.write().await;
//             guard.push(log);
//             Ok(1)
//         })
//     }
//     fn get_recent_logs<'a>(&'a self, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
//         Box::pin(async move {
//             let guard = self.logs.read().await;
//             let n = limit.max(0) as usize;
//             let start = guard.len().saturating_sub(n);
//             Ok(guard[start..].to_vec())
//         })
//     }
//     fn sum_total_tokens_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<u64>> {
//         Box::pin(async move {
//             let guard = self.logs.read().await;
//             let mut sum: u64 = 0;
//             for l in guard.iter() {
//                 if l.client_token.as_deref() == Some(token) {
//                     if let Some(v) = l.total_tokens { sum = sum.saturating_add(v as u64); }
//                 }
//             }
//             Ok(sum)
//         })
//     }
//     fn get_logs_by_client_token<'a>(&'a self, token: &'a str, limit: i32) -> BoxFuture<'a, rusqlite::Result<Vec<RequestLog>>> {
//         Box::pin(async move {
//             let guard = self.logs.read().await;
//             let mut v: Vec<RequestLog> = guard.iter().filter(|l| l.client_token.as_deref() == Some(token)).cloned().collect();
//             if (limit as usize) < v.len() { v = v[v.len() - (limit as usize)..].to_vec(); }
//             Ok(v)
//         })
//     }
//     fn log_provider_op<'a>(&'a self, _op: gateway::logging::types::ProviderOpLog) -> BoxFuture<'a, rusqlite::Result<i64>> {
//         Box::pin(async move { Ok(1) })
//     }
//     fn upsert_model_price<'a>(&'a self, provider: &'a str, model: &'a str, p_pm: f64, c_pm: f64, currency: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<()>> {
//         Box::pin(async move {
//             let mut g = self.prices.write().await;
//             g.insert((provider.to_string(), model.to_string()), (p_pm, c_pm, currency.map(|s| s.to_string())));
//             Ok(())
//         })
//     }
//     fn get_model_price<'a>(&'a self, provider: &'a str, model: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<(f64, f64, Option<String>)>>> {
//         Box::pin(async move {
//             let g = self.prices.read().await;
//             Ok(g.get(&(provider.to_string(), model.to_string())).cloned())
//         })
//     }
//     fn list_model_prices<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<(String, String, f64, f64, Option<String>)>>> {
//         Box::pin(async move {
//             let g = self.prices.read().await;
//             let mut out = Vec::new();
//             for ((prov, model), (p_pm, c_pm, cur)) in g.iter() {
//                 if provider.map(|p| p == prov.as_str()).unwrap_or(true) {
//                     out.push((prov.clone(), model.clone(), *p_pm, *c_pm, cur.clone()));
//                 }
//             }
//             Ok(out)
//         })
//     }
//     fn sum_spent_amount_by_client_token<'a>(&'a self, token: &'a str) -> BoxFuture<'a, rusqlite::Result<f64>> {
//         Box::pin(async move {
//             let logs = self.logs.read().await;
//             let prices = self.prices.read().await;
//             let mut sum = 0.0_f64;
//             for l in logs.iter() {
//                 if l.client_token.as_deref() != Some(token) { continue; }
//                 let (prov, model) = match (l.provider.as_ref(), l.model.as_ref()) { (Some(p), Some(m)) => (p, m), _ => continue };
//                 if let Some((p_pm, c_pm, _)) = prices.get(&(prov.clone(), model.clone())) {
//                     let p = l.prompt_tokens.unwrap_or(0) as f64 * *p_pm / 1_000_000.0;
//                     let c = l.completion_tokens.unwrap_or(0) as f64 * *c_pm / 1_000_000.0;
//                     sum += p + c;
//                 }
//             }
//             Ok(sum)
//         })
//     }
// }

// impl ModelCache for MemStore {
//     fn cache_models<'a>(&'a self, provider: &'a str, models: &'a [OpenAIModel]) -> BoxFuture<'a, rusqlite::Result<()> >{
//         Box::pin(async move {
//             let mut g = self.cached.write().await;
//             g.retain(|m| m.provider != provider);
//             let now = Utc::now();
//             for m in models {
//                 g.push(CachedModel{ id: m.id.clone(), provider: provider.to_string(), object: m.object.clone(), created: m.created, owned_by: m.owned_by.clone(), cached_at: now });
//             }
//             Ok(())
//         })
//     }
//     fn get_cached_models<'a>(&'a self, provider: Option<&'a str>) -> BoxFuture<'a, rusqlite::Result<Vec<CachedModel>>> {
//         Box::pin(async move {
//             let g = self.cached.read().await;
//             let v: Vec<CachedModel> = match provider {
//                 Some(p) => g.iter().filter(|m| m.provider == p).cloned().collect(),
//                 None => g.clone(),
//             };
//             Ok(v)
//         })
//     }
//     fn cache_models_append<'a>(&'a self, provider: &'a str, models: &'a [OpenAIModel]) -> BoxFuture<'a, rusqlite::Result<()>> {
//         Box::pin(async move {
//             let mut g = self.cached.write().await;
//             let now = Utc::now();
//             for m in models {
//                 g.push(CachedModel{ id: m.id.clone(), provider: provider.to_string(), object: m.object.clone(), created: m.created, owned_by: m.owned_by.clone(), cached_at: now });
//             }
//             Ok(())
//         })
//     }
//     fn remove_cached_models<'a>(&'a self, provider: &'a str, ids: &'a [String]) -> BoxFuture<'a, rusqlite::Result<()>> {
//         Box::pin(async move {
//             let mut g = self.cached.write().await;
//             let idset: std::collections::HashSet<&String> = ids.iter().collect();
//             g.retain(|m| !(m.provider == provider && idset.contains(&m.id)));
//             Ok(())
//         })
//     }
// }

// impl ProviderStore for MemStore {
//     fn insert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<bool>> {
//         Box::pin(async move {
//             let mut g = self.providers.write().await;
//             let existed = g.contains_key(&provider.name);
//             g.insert(provider.name.clone(), provider.clone());
//             Ok(!existed)
//         })
//     }
//     fn upsert_provider<'a>(&'a self, provider: &'a Provider) -> BoxFuture<'a, rusqlite::Result<()>> {
//         Box::pin(async move {
//             let mut g = self.providers.write().await;
//             g.insert(provider.name.clone(), provider.clone());
//             Ok(())
//         })
//     }
//     fn provider_exists<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
//         Box::pin(async move { Ok(self.providers.read().await.contains_key(name)) })
//     }
//     fn get_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<Option<Provider>>> {
//         Box::pin(async move { Ok(self.providers.read().await.get(name).cloned()) })
//     }
//     fn list_providers<'a>(&'a self) -> BoxFuture<'a, rusqlite::Result<Vec<Provider>>> {
//         Box::pin(async move { Ok(self.providers.read().await.values().cloned().collect()) })
//     }
//     fn delete_provider<'a>(&'a self, name: &'a str) -> BoxFuture<'a, rusqlite::Result<bool>> {
//         Box::pin(async move {
//             let mut provs = self.providers.write().await;
//             let existed = provs.remove(name).is_some();
//             // cascade-like cleanup
//             self.provider_keys.write().await.remove(name);
//             self.cached.write().await.retain(|m| m.provider != name);
//             Ok(existed)
//         })
//     }
//     fn get_provider_keys<'a>(&'a self, provider: &'a str, _strategy: &'a Option<gateway::config::settings::KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<Vec<String>>> {
//         Box::pin(async move {
//             Ok(self.provider_keys.read().await.get(provider).cloned().unwrap_or_default())
//         })
//     }
//     fn add_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, _strategy: &'a Option<gateway::config::settings::KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<()>> {
//         Box::pin(async move {
//             let mut g = self.provider_keys.write().await;
//             g.entry(provider.to_string()).or_default();
//             if let Some(v) = g.get_mut(provider) {
//                 if !v.iter().any(|k| k == key) { v.push(key.to_string()); }
//             }
//             Ok(())
//         })
//     }
//     fn remove_provider_key<'a>(&'a self, provider: &'a str, key: &'a str, _strategy: &'a Option<gateway::config::settings::KeyLogStrategy>) -> BoxFuture<'a, rusqlite::Result<bool>> {
//         Box::pin(async move {
//             let mut g = self.provider_keys.write().await;
//             if let Some(v) = g.get_mut(provider) {
//                 let before = v.len();
//                 v.retain(|k| k != key);
//                 return Ok(v.len() < before);
//             }
//             Ok(false)
//         })
//     }
// }

// #[derive(Clone, Default)]
// struct MemTokenStore {
//     tokens: Arc<tokio::sync::RwLock<HashMap<String, AdminToken>>>,
// }

// #[async_trait]
// impl TokenStore for MemTokenStore {
//     async fn create_token(&self, payload: CreateTokenPayload) -> Result<AdminToken, gateway::error::GatewayError> {
//         let token = payload.token.unwrap_or_else(|| {
//             use rand::Rng; use rand::distr::Alphanumeric;
//             let rng = rand::rng();
//             rng.sample_iter(&Alphanumeric).take(40).map(char::from).collect::<String>()
//         });
//         let now = Utc::now();
//         let t = AdminToken {
//             token: token.clone(),
//             allowed_models: payload.allowed_models,
//             max_tokens: payload.max_tokens,
//             max_amount: payload.max_amount,
//             enabled: payload.enabled,
//             expires_at: payload.expires_at.and_then(|s| gateway::logging::time::parse_beijing_string(&s).ok()),
//             created_at: now,
//             amount_spent: 0.0,
//             prompt_tokens_spent: 0,
//             completion_tokens_spent: 0,
//             total_tokens_spent: 0,
//         };
//         self.tokens.write().await.insert(token.clone(), t.clone());
//         Ok(t)
//     }
//     async fn update_token(&self, token: &str, payload: UpdateTokenPayload) -> Result<Option<AdminToken>, gateway::error::GatewayError> {
//         let mut g = self.tokens.write().await;
//         if let Some(t) = g.get_mut(token) {
//             if let Some(v) = payload.allowed_models { t.allowed_models = Some(v); }
//             if let Some(v) = payload.max_tokens { t.max_tokens = v; }
//             if let Some(v) = payload.max_amount { t.max_amount = v; }
//             if let Some(v) = payload.enabled { t.enabled = v; }
//             if let Some(v) = payload.expires_at { t.expires_at = v.and_then(|s| gateway::logging::time::parse_beijing_string(&s).ok()); }
//             return Ok(Some(t.clone()));
//         }
//         Ok(None)
//     }
//     async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, gateway::error::GatewayError> {
//         let mut g = self.tokens.write().await;
//         if let Some(t) = g.get_mut(token) { t.enabled = enabled; return Ok(true); }
//         Ok(false)
//     }
//     async fn get_token(&self, token: &str) -> Result<Option<AdminToken>, gateway::error::GatewayError> {
//         Ok(self.tokens.read().await.get(token).cloned())
//     }
//     async fn list_tokens(&self) -> Result<Vec<AdminToken>, gateway::error::GatewayError> {
//         Ok(self.tokens.read().await.values().cloned().collect())
//     }
//     async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), gateway::error::GatewayError> {
//         if let Some(t) = self.tokens.write().await.get_mut(token) { t.amount_spent += delta; }
//         Ok(())
//     }
//     async fn add_usage_spent(&self, token: &str, prompt: i64, completion: i64, total: i64) -> Result<(), gateway::error::GatewayError> {
//         if let Some(t) = self.tokens.write().await.get_mut(token) {
//             t.prompt_tokens_spent += prompt;
//             t.completion_tokens_spent += completion;
//             t.total_tokens_spent += total;
//         }
//         Ok(())
//     }
// }

// fn build_bench_app() -> (Router<Arc<AppState>>, String, String) {
//     // Minimal settings to avoid any external IO
//     let settings = Settings {
//         load_balancing: LoadBalancing { strategy: BalanceStrategy::FirstAvailable },
//         server: ServerConfig { host: "127.0.0.1".into(), port: 0, admin_secret: None },
//         logging: LoggingConfig { database_path: "".into(), key_log_strategy: None, pg_url: None, pg_schema: None, pg_pool_size: None },
//     };

//     let mem_store = MemStore::default();
//     let token_store = MemTokenStore::default();

//     // Preload: provider + key + cached models + prices
//     let rt = tokio::runtime::Runtime::new().unwrap();
//     rt.block_on(async {
//         let _ = mem_store.upsert_provider(&Provider { name: "openai".into(), api_type: ProviderType::OpenAI, base_url: "https://api.openai.com".into(), api_keys: vec![], models_endpoint: None }).await;
//         let _ = mem_store.add_provider_key("openai", "sk-bench", &None).await;
//         let models = vec![OpenAIModel { id: "gpt-4o-mini".into(), object: "model".into(), created: Utc::now().timestamp() as u64, owned_by: "openai".into() }];
//         let _ = mem_store.cache_models("openai", &models).await;
//         let _ = mem_store.upsert_model_price("openai", "gpt-4o-mini", 5.0, 15.0, Some("USD")).await;
//     });

//     // Admin identity and user token
//     let admin_tok = "admin-bench-token".to_string();
//     let user_tok = {
//         let t = tokio::runtime::Runtime::new().unwrap().block_on(async {
//             token_store.create_token(CreateTokenPayload { token: Some("user-bench-token".into()), allowed_models: None, max_tokens: None, max_amount: Some(100.0), enabled: true, expires_at: None }).await.unwrap()
//         });
//         t.token
//     };

//     // Construct AppState wired to in-memory stores
//     let app_state = AppState {
//         config: settings,
//         log_store: Arc::new(mem_store.clone()),
//         model_cache: Arc::new(mem_store.clone()),
//         providers: Arc::new(mem_store),
//         token_store: Arc::new(token_store),
//         admin_identity_token: admin_tok.clone(),
//     };

//     let app = handlers::routes().with_state(Arc::new(app_state));
//     (app, admin_tok, user_tok)
// }

// fn bench_endpoints(c: &mut Criterion) {
//     let (app, admin_token, user_token) = build_bench_app();
//     let rt = tokio::runtime::Runtime::new().unwrap();

//     // GET /v1/models (admin)
//     let mut group = c.benchmark_group("endpoints");
//     group.measurement_time(Duration::from_secs(5));

//     group.bench_function(BenchmarkId::new("GET /v1/models", "admin"), |b| {
//         b.to_async(&rt).iter(|| async {
//             let req = Request::builder()
//                 .method("GET")
//                 .uri("/v1/models")
//                 .header("authorization", format!("Bearer {}", admin_token))
//                 .body(Body::empty())
//                 .unwrap();
//             let resp = app.clone().oneshot(req).await.unwrap();
//             assert_eq!(resp.status(), StatusCode::OK);
//             black_box(resp);
//         })
//     });

//     // GET /v1/token/balance (user token)
//     group.bench_function(BenchmarkId::new("GET /v1/token/balance", "user"), |b| {
//         b.to_async(&rt).iter(|| async {
//             let req = Request::builder()
//                 .method("GET")
//                 .uri("/v1/token/balance")
//                 .header("authorization", format!("Bearer {}", user_token))
//                 .body(Body::empty())
//                 .unwrap();
//             let resp = app.clone().oneshot(req).await.unwrap();
//             assert_eq!(resp.status(), StatusCode::OK);
//             black_box(resp);
//         })
//     });

//     // GET /v1/token/usage (user token)
//     group.bench_function(BenchmarkId::new("GET /v1/token/usage", "user"), |b| {
//         b.to_async(&rt).iter(|| async {
//             let req = Request::builder()
//                 .method("GET")
//                 .uri("/v1/token/usage?limit=10")
//                 .header("authorization", format!("Bearer {}", user_token))
//                 .body(Body::empty())
//                 .unwrap();
//             let resp = app.clone().oneshot(req).await.unwrap();
//             assert_eq!(resp.status(), StatusCode::OK);
//             black_box(resp);
//         })
//     });

//     group.finish();
// }

// criterion_group!(benches, bench_endpoints);
// criterion_main!(benches);

fn main() {
    todo!()
}
