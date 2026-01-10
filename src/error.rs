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
