use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::auth::{AdminIdentity, ensure_admin};
use crate::error::GatewayError;
use crate::logging::types::RequestLog;
use crate::server::AppState;
use crate::server::request_logging::log_simple_request;

const DEFAULT_WINDOW_MINUTES: i64 = 60;
const DEFAULT_INTERVAL_MINUTES: i64 = 5;
const MAX_FETCH_LIMIT: i32 = 5000;

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SeriesQuery {
    #[serde(default)]
    pub window_minutes: Option<i64>,
    #[serde(default)]
    pub interval_minutes: Option<i64>,
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

fn filter_logs<'a>(logs: &'a [RequestLog], since: DateTime<Utc>) -> Vec<&'a RequestLog> {
    logs.iter().filter(|log| log.timestamp >= since).collect()
}

fn aggregate_summary(logs: &[&RequestLog], window_minutes: i64) -> MetricsSummary {
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
        if let Some(provider) = &log.provider {
            *provider_counts.entry(provider.clone()).or_insert(0) += 1;
        }
        if let Some(model) = &log.model {
            *model_counts.entry(model.clone()).or_insert(0) += 1;
        }
        if let Some(client) = &log.client_token {
            clients.entry(client.clone());
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

fn build_series(logs: &[&RequestLog], window_minutes: i64, interval_minutes: i64) -> MetricsSeries {
    let now = Utc::now();
    let interval = Duration::minutes(interval_minutes);
    let buckets = ((window_minutes + interval_minutes - 1) / interval_minutes).max(1) as usize;
    let mut points = Vec::with_capacity(buckets);

    for i in 0..buckets {
        let bucket_end =
            now - Duration::minutes((window_minutes - interval_minutes * i as i64).max(0));
        let bucket_start = bucket_end - interval;
        let mut count = 0usize;
        let mut error = 0usize;
        let mut latency_sum: i64 = 0;
        let mut amount_sum = 0.0;
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
        });
    }

    MetricsSeries {
        window_minutes,
        interval_minutes,
        points,
        generated_at: now.to_rfc3339(),
    }
}

pub async fn summary(
    State(app_state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<MetricsQuery>,
) -> Result<Json<MetricsSummary>, GatewayError> {
    let identity = ensure_admin(&headers, &app_state).await?;
    let window_minutes = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);
    let since = Utc::now() - Duration::minutes(window_minutes);
    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, since);
    let summary = aggregate_summary(&filtered, window_minutes);

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
    let window_minutes = q
        .window_minutes
        .unwrap_or(DEFAULT_WINDOW_MINUTES)
        .clamp(1, 24 * 60);
    let interval_minutes = q
        .interval_minutes
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
        .clamp(1, window_minutes.max(1));
    let since = Utc::now() - Duration::minutes(window_minutes);
    let fetch_limit = fetch_limit(window_minutes);
    let logs = app_state
        .log_store
        .get_recent_logs(fetch_limit)
        .await
        .map_err(GatewayError::Db)?;
    let filtered = filter_logs(&logs, since);
    let series = build_series(&filtered, window_minutes, interval_minutes);

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
