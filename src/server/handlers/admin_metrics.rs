use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use super::auth::{AdminIdentity, ensure_admin};
use crate::error::GatewayError;
use crate::logging::time::BEIJING_OFFSET;
use crate::logging::types::RequestLog;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;

const DEFAULT_WINDOW_MINUTES: i64 = 60;
const DEFAULT_INTERVAL_MINUTES: i64 = 5;
const MAX_FETCH_LIMIT: i32 = 5000;
const TARGET_METHOD: &str = "POST";
const TARGET_PATH: &str = "/v1/chat/completions";

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SeriesQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub interval_minutes: Option<i64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MetricsSummary {
    pub window_minutes: i64,
    pub total_requests: usize,
    pub success_requests: usize,
    pub error_requests: usize,
    pub error_rate: f64,
    pub average_latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p95_latency_ms: Option<f64>,
    pub total_amount_spent: f64,
    pub total_tokens: u64,
    pub unique_clients: usize,
    pub top_providers: Vec<TopItem>,
    pub top_models: Vec<TopItem>,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub available_dates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TopItem {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct SeriesPoint {
    pub bucket_start: String,
    pub requests: usize,
    pub errors: usize,
    pub average_latency_ms: f64,
    pub amount_spent: f64,
    pub total_tokens: u64,
}

#[derive(Debug, Serialize)]
pub struct MetricsSeries {
    pub window_minutes: i64,
    pub interval_minutes: i64,
    pub points: Vec<SeriesPoint>,
    pub generated_at: String,
}

fn identity_label(identity: &AdminIdentity) -> &'static str {
    match identity {
        AdminIdentity::TuiSession(_) => "tui_session",
        AdminIdentity::WebSession(_) => "web_session",
    }
}

fn fetch_limit(window_minutes: i64) -> i32 {
    let approx = (window_minutes.max(1) * 50) as i32;
    approx.clamp(500, MAX_FETCH_LIMIT)
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn start_of_day_utc(date: NaiveDate) -> DateTime<Utc> {
    BEIJING_OFFSET
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn end_of_day_exclusive_utc(date: NaiveDate) -> DateTime<Utc> {
    start_of_day_utc(date) + Duration::days(1)
}

fn enumerate_available_dates(min: DateTime<Utc>, max: DateTime<Utc>) -> Vec<String> {
    let start = min.with_timezone(&BEIJING_OFFSET).date_naive();
    let end = max.with_timezone(&BEIJING_OFFSET).date_naive();
    let mut current = start;
    let mut out = Vec::new();
    while current <= end {
        out.push(current.format("%Y-%m-%d").to_string());
        current = current.succ_opt().unwrap();
    }
    out
}

fn resolve_bounds(
    available_dates: &[String],
    query_start: Option<&str>,
    query_end: Option<&str>,
    default_window_minutes: i64,
) -> (
    DateTime<Utc>,
    DateTime<Utc>,
    i64,
    Option<String>,
    Option<String>,
) {
    if available_dates.is_empty() {
        let until = Utc::now();
        let since = until - Duration::minutes(default_window_minutes);
        return (since, until, default_window_minutes, None, None);
    }

    let first_date = parse_date(&available_dates[0]).unwrap();
    let last_date = parse_date(&available_dates[available_dates.len() - 1]).unwrap();

    let start_date = query_start
        .and_then(parse_date)
        .filter(|d| *d >= first_date && *d <= last_date)
        .unwrap_or(first_date);
    let end_date = query_end
        .and_then(parse_date)
        .filter(|d| *d >= start_date && *d <= last_date)
        .unwrap_or(last_date);

    let since = start_of_day_utc(start_date);
    let until = end_of_day_exclusive_utc(end_date);
    let window_minutes = (until - since).num_minutes().max(1);

    (
        since,
        until,
        window_minutes,
        Some(start_date.format("%Y-%m-%d").to_string()),
        Some(end_date.format("%Y-%m-%d").to_string()),
    )
}

fn filter_logs(
    logs: &[RequestLog],
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Vec<&RequestLog> {
    logs.iter()
        .filter(|log| {
            log.method.eq_ignore_ascii_case(TARGET_METHOD)
                && log.path.as_str() == TARGET_PATH
                && since.map_or(true, |start| log.timestamp >= start)
                && until.map_or(true, |end| log.timestamp < end)
        })
        .collect()
}

fn aggregate_summary(
    logs: &[&RequestLog],
    window_minutes: i64,
    start_date: Option<String>,
    end_date: Option<String>,
    available_dates: Vec<String>,
) -> MetricsSummary {
    let total_requests = logs.len();
    let (mut success_requests, mut error_requests) = (0usize, 0usize);
    let mut latencies = Vec::with_capacity(total_requests);
    let mut total_latency: i64 = 0;
    let mut total_amount = 0.0;
    let mut total_tokens: u64 = 0;
    let mut provider_counts: HashMap<String, usize> = HashMap::new();
    let mut model_counts: HashMap<String, usize> = HashMap::new();
    let mut clients: HashMap<String, ()> = HashMap::new();

    for log in logs {
        if log.status_code < 400 {
            success_requests += 1;
        } else {
            error_requests += 1;
        }
        total_latency += log.response_time_ms;
        latencies.push(log.response_time_ms);
        if let Some(amount) = log.amount_spent {
            total_amount += amount;
        }
        if let Some(tokens) = log.total_tokens {
            total_tokens += tokens as u64;
        }
        let provider_name = log
            .provider
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("未知供应商");
        *provider_counts
            .entry(provider_name.to_string())
            .or_insert(0) += 1;

        let model_label = normalize_model_label(log.provider.as_deref(), log.model.as_deref());
        *model_counts.entry(model_label).or_insert(0) += 1;
        if let Some(client) = &log.client_token {
            clients.entry(client.clone()).or_insert(());
        }
    }

    let error_rate = if total_requests == 0 {
        0.0
    } else {
        error_requests as f64 / total_requests as f64
    };
    let avg_latency = if total_requests == 0 {
        0.0
    } else {
        total_latency as f64 / total_requests as f64
    };

    latencies.sort();
    let p95_latency_ms = if total_requests == 0 {
        None
    } else {
        let idx = ((total_requests as f64) * 0.95).ceil() as usize;
        let pos = idx.clamp(1, total_requests) - 1;
        latencies.get(pos).map(|v| *v as f64)
    };

    let top_providers = top_items(provider_counts, 5);
    let top_models = top_items(model_counts, 5);

    MetricsSummary {
        window_minutes,
        total_requests,
        success_requests,
        error_requests,
        error_rate,
        average_latency_ms: avg_latency,
        p95_latency_ms,
        total_amount_spent: total_amount,
        total_tokens,
        unique_clients: clients.len(),
        top_providers,
        top_models,
        generated_at: Utc::now().to_rfc3339(),
        start_date,
        end_date,
        available_dates,
    }
}

fn top_items(map: HashMap<String, usize>, limit: usize) -> Vec<TopItem> {
    let mut items: Vec<_> = map.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1));
    items
        .into_iter()
        .take(limit)
        .map(|(name, count)| TopItem { name, count })
        .collect()
}

fn build_series(
    logs: &[&RequestLog],
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    interval_minutes: i64,
) -> MetricsSeries {
    let interval = Duration::minutes(interval_minutes);
    let total_minutes = (until - since).num_minutes().max(interval_minutes);
    let buckets = ((total_minutes + interval_minutes - 1) / interval_minutes) as usize;
    let mut points = Vec::with_capacity(buckets);

    for i in 0..buckets {
        let bucket_start = since + Duration::minutes(interval_minutes * i as i64);
        let bucket_end = (bucket_start + interval).min(until);
        let mut count = 0usize;
        let mut error = 0usize;
        let mut latency_sum: i64 = 0;
        let mut amount_sum = 0.0;
        let mut token_sum: u64 = 0;
        for log in logs {
            if log.timestamp >= bucket_start && log.timestamp < bucket_end {
                count += 1;
                if log.status_code >= 400 {
                    error += 1;
                }
                latency_sum += log.response_time_ms;
                if let Some(a) = log.amount_spent {
                    amount_sum += a;
                }
                if let Some(tokens) = log.total_tokens {
                    token_sum += tokens as u64;
                }
            }
        }
        let avg_latency = if count == 0 {
            0.0
        } else {
            latency_sum as f64 / count as f64
        };
        points.push(SeriesPoint {
            bucket_start: bucket_start.to_rfc3339(),
            requests: count,
            errors: error,
            average_latency_ms: avg_latency,
            amount_spent: amount_sum,
            total_tokens: token_sum,
        });
    }

    MetricsSeries {
        window_minutes: total_minutes,
        interval_minutes,
        points,
        generated_at: Utc::now().to_rfc3339(),
    }
}

fn normalize_model_label(provider: Option<&str>, model: Option<&str>) -> String {
    let provider = provider.unwrap_or("");
    let model = model.unwrap_or("未知模型");
    if model.contains('/') {
        // 如果 model 已带前缀，且与 provider 重复，则保持原样，避免重复前缀
        let mut parts = model.splitn(2, '/');
        let first = parts.next().unwrap_or("");
        if !provider.is_empty() && first.eq_ignore_ascii_case(provider) {
            model.to_string()
        } else {
            model.to_string()
        }
    } else if provider.is_empty() {
        model.to_string()
    } else {
        format!("{}/{}", provider, model)
    }
}

#[derive(Debug, Deserialize)]
pub struct ModelsDistributionQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ModelCountItem {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct ModelsDistributionResponse {
    pub items: Vec<ModelCountItem>,
    pub generated_at: String,
}

pub async fn models_distribution(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<ModelsDistributionQuery>,
) -> Result<Json<ModelsDistributionResponse>, GatewayError> {
    let identity = ensure_admin(&headers, &app_state).await?;
    let date_range = app_state
        .log_store
        .get_request_log_date_range(TARGET_METHOD, TARGET_PATH)
        .await
        .map_err(GatewayError::Db)?;
    let available_dates = date_range
        .map(|(min, max)| enumerate_available_dates(min, max))
        .unwrap_or_default();

    let default_window = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);
    let (since, until, window_minutes, _, _) = resolve_bounds(
        &available_dates,
        q.start_date.as_deref(),
        q.end_date.as_deref(),
        default_window,
    );

    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, Some(since), Some(until));

    let mut counts: HashMap<String, usize> = HashMap::new();
    for log in filtered {
        let label = normalize_model_label(log.provider.as_deref(), log.model.as_deref());
        *counts.entry(label).or_insert(0) += 1;
    }
    let limit = q.limit.unwrap_or(8).max(1);
    let mut items: Vec<_> = counts.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1));
    let items = items
        .into_iter()
        .take(limit)
        .map(|(name, count)| ModelCountItem { name, count })
        .collect();

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/metrics/models-distribution",
        "admin_metrics_models_distribution",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(ModelsDistributionResponse {
        items,
        generated_at: Utc::now().to_rfc3339(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct SeriesModelsQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub interval_minutes: Option<i64>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SeriesModelPoint {
    pub bucket_start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MetricsSeriesModels {
    pub window_minutes: i64,
    pub interval_minutes: i64,
    pub items: Vec<SeriesModelPoint>,
    pub generated_at: String,
}

pub async fn series_models(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<SeriesModelsQuery>,
) -> Result<Json<MetricsSeriesModels>, GatewayError> {
    let identity = ensure_admin(&headers, &app_state).await?;
    let date_range = app_state
        .log_store
        .get_request_log_date_range(TARGET_METHOD, TARGET_PATH)
        .await
        .map_err(GatewayError::Db)?;
    let available_dates = date_range
        .map(|(min, max)| enumerate_available_dates(min, max))
        .unwrap_or_default();

    let default_window = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);
    let interval_minutes = q
        .interval_minutes
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
        .clamp(1, default_window.max(1));

    let (since, until, window_minutes, _, _) = resolve_bounds(
        &available_dates,
        q.start_date.as_deref(),
        q.end_date.as_deref(),
        default_window,
    );

    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, Some(since), Some(until));

    let interval = Duration::minutes(interval_minutes);
    let total_minutes = (until - since).num_minutes().max(interval_minutes);
    let buckets = ((total_minutes + interval_minutes - 1) / interval_minutes) as usize;
    let mut items = Vec::with_capacity(buckets);

    for i in 0..buckets {
        let bucket_start = since + Duration::minutes(interval_minutes * i as i64);
        let bucket_end = (bucket_start + interval).min(until);
        let mut model_counts: HashMap<String, usize> = HashMap::new();
        for log in &filtered {
            if log.timestamp >= bucket_start && log.timestamp < bucket_end {
                let label = normalize_model_label(log.provider.as_deref(), log.model.as_deref());
                *model_counts.entry(label).or_insert(0) += 1;
            }
        }
        let top_model = model_counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1))
            .map(|(name, _)| name);
        items.push(SeriesModelPoint {
            bucket_start: bucket_start.to_rfc3339(),
            top_model,
        });
    }

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/metrics/series-models",
        "admin_metrics_series_models",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(MetricsSeriesModels {
        window_minutes: (until - since).num_minutes(),
        interval_minutes,
        items,
        generated_at: Utc::now().to_rfc3339(),
    }))
}

pub async fn summary(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<MetricsSummary>, GatewayError> {
    let identity = ensure_admin(&headers, &app_state).await?;
    let date_range = app_state
        .log_store
        .get_request_log_date_range(TARGET_METHOD, TARGET_PATH)
        .await
        .map_err(GatewayError::Db)?;
    let available_dates = date_range
        .map(|(min, max)| enumerate_available_dates(min, max))
        .unwrap_or_default();

    let default_window = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);

    let (since, until, window_minutes, start_date, end_date) = resolve_bounds(
        &available_dates,
        q.start_date.as_deref(),
        q.end_date.as_deref(),
        default_window,
    );

    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, Some(since), Some(until));
    let summary = aggregate_summary(
        &filtered,
        window_minutes,
        start_date.clone(),
        end_date.clone(),
        available_dates.clone(),
    );

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/metrics/summary",
        "admin_metrics_summary",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(summary))
}

pub async fn series(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<SeriesQuery>,
) -> Result<Json<MetricsSeries>, GatewayError> {
    let identity = ensure_admin(&headers, &app_state).await?;
    let date_range = app_state
        .log_store
        .get_request_log_date_range(TARGET_METHOD, TARGET_PATH)
        .await
        .map_err(GatewayError::Db)?;
    let available_dates = date_range
        .map(|(min, max)| enumerate_available_dates(min, max))
        .unwrap_or_default();

    let default_window = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);
    let interval_minutes = q
        .interval_minutes
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
        .clamp(1, default_window.max(1));

    let (since, until, window_minutes, _, _) = resolve_bounds(
        &available_dates,
        q.start_date.as_deref(),
        q.end_date.as_deref(),
        default_window,
    );

    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, Some(since), Some(until));
    let series = build_series(&filtered, since, until, interval_minutes);

    log_simple_request(
        &app_state,
        Utc::now(),
        "GET",
        "/admin/metrics/series",
        "admin_metrics_series",
        None,
        None,
        Some(identity_label(&identity)),
        200,
        None,
    )
    .await;

    Ok(Json(series))
}
