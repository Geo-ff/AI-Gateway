use thiserror::Error;

use crate::routing::load_balancer::BalanceError;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

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
}

pub type Result<T> = std::result::Result<T, GatewayError>;

