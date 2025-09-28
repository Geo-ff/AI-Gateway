use axum::http::HeaderMap;

use crate::config::settings::{KeyLogStrategy, LoggingConfig};

// HTTP helpers
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

// Map provided token to a safe value for logging (admin token masked as literal label)
pub fn token_for_log<'a>(provided: Option<&'a str>, admin_identity_token: &'a str) -> Option<&'a str> {
    provided.map(|tok| if tok == admin_identity_token { "admin_token" } else { tok })
}

// Key masking and hint utilities (DRY across modules)
pub fn mask_key(key: &str) -> String {
    if key.len() <= 8 { return "****".to_string(); }
    let (start, end) = (&key[..4], &key[key.len()-4..]);
    format!("{}****{}", start, end)
}

pub fn api_key_hint(cfg: &LoggingConfig, key: &str) -> Option<String> {
    match cfg.key_log_strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

pub fn key_display_hint(strategy: &Option<KeyLogStrategy>, key: &str) -> Option<String> {
    match strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::None => None,
        KeyLogStrategy::Plain => Some(key.to_string()),
        KeyLogStrategy::Masked => Some(mask_key(key)),
    }
}

