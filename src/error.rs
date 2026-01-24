use thiserror::Error;

use crate::routing::load_balancer::BalanceError;
use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Balance error: {0}")]
    Balance(#[from] BalanceError),

    #[error("Time parse error: {0}")]
    TimeParse(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
}

pub type Result<T> = std::result::Result<T, GatewayError>;

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl GatewayError {
    fn user_message(&self) -> String {
        match self {
            GatewayError::Http(err) => format_reqwest_error(err),
            GatewayError::TimeParse(s)
            | GatewayError::Config(s)
            | GatewayError::NotFound(s)
            | GatewayError::RateLimited(s)
            | GatewayError::Unauthorized(s)
            | GatewayError::Forbidden(s) => s.clone(),
            _ => self.to_string(),
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            GatewayError::Balance(BalanceError::NoProvidersAvailable)
            | GatewayError::Balance(BalanceError::NoApiKeysAvailable) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            GatewayError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            GatewayError::Forbidden(_) => StatusCode::FORBIDDEN,
            GatewayError::Http(_) => StatusCode::BAD_GATEWAY,
            GatewayError::Config(_) => StatusCode::BAD_REQUEST,
            GatewayError::NotFound(_) => StatusCode::NOT_FOUND,
            GatewayError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            GatewayError::Http(_) => "http_error",
            GatewayError::Json(_) => "json_error",
            GatewayError::Toml(_) => "toml_error",
            GatewayError::Db(_) => "db_error",
            GatewayError::Io(_) => "io_error",
            GatewayError::Balance(_) => "balance_error",
            GatewayError::TimeParse(_) => "time_parse_error",
            GatewayError::Config(_) => "config_error",
            GatewayError::NotFound(_) => "not_found",
            GatewayError::RateLimited(_) => "rate_limited",
            GatewayError::Unauthorized(_) => "unauthorized",
            GatewayError::Forbidden(_) => "forbidden",
        }
    }
}

fn format_reqwest_error(err: &reqwest::Error) -> String {
    use std::error::Error as _;

    let mut msg = err.to_string();

    // Append a compact source chain to avoid losing the actual root cause (DNS/TLS/proxy/etc).
    let mut chain = Vec::new();
    let mut cur = err.source();
    while let Some(e) = cur {
        let s = e.to_string();
        if !s.trim().is_empty() {
            chain.push(s);
        }
        cur = e.source();
    }
    if !chain.is_empty() {
        msg.push_str("; caused by: ");
        msg.push_str(&chain.join(" -> "));
    }

    // Actionable hint: reqwest uses HTTP(S)_PROXY by default; local proxy failures often look like
    // "error sending request". If this URL would be bypassed by our proxy rule, call it out.
    if let Some(url) = err.url().map(|u| u.as_str()) {
        if crate::http_client::should_bypass_proxy_for_url(url) {
            msg.push_str(
                "; hint: proxy env vars detected; consider setting NO_PROXY for this host or disabling HTTP(S)_PROXY for the gateway process",
            );
        }
    }

    msg
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status_code();
        let body = ErrorBody {
            code: self.code(),
            message: self.user_message(),
        };
        (status, Json(body)).into_response()
    }
}
