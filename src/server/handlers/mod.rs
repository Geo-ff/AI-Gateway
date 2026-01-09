use axum::{
    Router,
    routing::{delete, get, post},
};
use std::sync::Arc;

use crate::server::AppState;

mod admin_logs;
mod admin_metrics;
mod admin_prices;
mod client_tokens;
mod admin_users;
mod auth;
mod auth_jwt;
mod auth_keys;
mod auth_login;
mod auth_tui;
mod auth_tui_admin;
mod cache;
mod chat;
mod models;
mod provider_keys;
mod providers;
mod token_info;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Auth for Web
        .route("/auth/tui/challenge", post(auth_tui::challenge))
        .route("/auth/tui/verify", post(auth_tui::verify))
        // Admin key management
        .route(
            "/auth/keys",
            get(auth_keys::list_keys).post(auth_keys::add_key),
        )
        .route("/auth/keys/{fingerprint}", delete(auth_keys::delete_key))
        // TUI sessions management
        .route("/auth/tui/sessions", get(auth_tui_admin::list_tui_sessions))
        .route(
            "/auth/tui/sessions/{session_id}/revoke",
            post(auth_tui_admin::revoke_tui_session),
        )
        .route("/auth/login-codes", post(auth_login::create_login_code))
        .route(
            "/auth/login-codes/status",
            get(auth_login::current_code_status),
        )
        .route("/auth/register", post(auth_jwt::register))
        .route("/auth/login", post(auth_jwt::login))
        .route("/auth/refresh", post(auth_jwt::refresh))
        .route("/auth/me", get(auth_jwt::me))
        .route("/auth/code/redeem", post(auth_login::redeem_code))
        .route("/auth/session", get(auth_login::get_session))
        .route("/auth/logout", post(auth_login::logout))
        .route("/v1/chat/completions", post(chat::chat_completions))
        .route("/v1/models", get(models::list_models))
        .route("/models/{provider}", get(models::list_provider_models))
        .route(
            "/models/{provider}/cache",
            post(cache::update_provider_cache).delete(cache::delete_provider_cache),
        )
        .route("/admin/models/cache", get(cache::list_cached_models))
        .route(
            "/providers/{provider}/keys",
            get(provider_keys::list_provider_keys)
                .post(provider_keys::add_provider_key)
                .delete(provider_keys::delete_provider_key),
        )
        .route(
            "/providers/{provider}/keys/raw",
            get(provider_keys::list_provider_keys_raw),
        )
        .route(
            "/providers/{provider}/keys/batch",
            post(provider_keys::add_provider_keys_batch)
                .delete(provider_keys::delete_provider_keys_batch),
        )
        .route(
            "/providers",
            get(providers::list_providers).post(providers::create_provider),
        )
        .route(
            "/providers/{provider}",
            get(providers::get_provider)
                .put(providers::update_provider)
                .delete(providers::delete_provider),
        )
        .route(
            "/admin/tokens",
            get(client_tokens::list_tokens).post(client_tokens::create_token),
        )
        .route(
            "/admin/tokens/{id}",
            get(client_tokens::get_token)
                .put(client_tokens::update_token)
                .delete(client_tokens::delete_token),
        )
        .route(
            "/admin/tokens/{id}/toggle",
            post(client_tokens::toggle_token),
        )
        .route(
            "/admin/users",
            get(admin_users::list_users).post(admin_users::create_user),
        )
        .route(
            "/admin/users/{id}",
            get(admin_users::get_user)
                .put(admin_users::update_user)
                .delete(admin_users::delete_user),
        )
        .route(
            "/admin/model-prices",
            post(admin_prices::upsert_model_price).get(admin_prices::list_model_prices),
        )
        .route(
            "/admin/model-prices/{provider}/{model}",
            get(admin_prices::get_model_price),
        )
        .route("/admin/metrics/summary", get(admin_metrics::summary))
        .route("/admin/metrics/series", get(admin_metrics::series))
        .route(
            "/admin/metrics/models-distribution",
            get(admin_metrics::models_distribution),
        )
        .route(
            "/admin/metrics/series-models",
            get(admin_metrics::series_models),
        )
        .route("/admin/logs/requests", get(admin_logs::list_request_logs))
        .route(
            "/admin/logs/chat-completions",
            get(admin_logs::list_chat_completion_logs),
        )
        .route(
            "/admin/logs/operations",
            get(admin_logs::list_operation_logs),
        )
        .route("/v1/token/balance", get(token_info::token_balance))
        .route("/v1/token/usage", get(token_info::token_usage))
}
