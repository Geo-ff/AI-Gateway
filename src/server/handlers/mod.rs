use axum::{routing::{get, post}, Router};
use std::sync::Arc;

use crate::server::AppState;

mod chat;
mod models;
mod cache;
mod provider_keys;
mod providers;
mod admin_tokens;
mod admin_prices;
mod token_info;
mod auth;
mod auth_login;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // Auth for Web
        .route("/auth/login-codes", post(auth_login::create_login_code))
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
        .route(
            "/providers/{provider}/keys",
            get(provider_keys::list_provider_keys)
                .post(provider_keys::add_provider_key)
                .delete(provider_keys::delete_provider_key),
        )
        .route("/providers", get(providers::list_providers).post(providers::create_provider))
        .route(
            "/providers/{provider}",
            get(providers::get_provider).put(providers::update_provider).delete(providers::delete_provider),
        )
        .route("/admin/tokens", get(admin_tokens::list_tokens).post(admin_tokens::create_token))
        .route(
            "/admin/tokens/{token}",
            get(admin_tokens::get_token).put(admin_tokens::update_token),
        )
        .route(
            "/admin/tokens/{token}/toggle",
            post(admin_tokens::toggle_token),
        )
        .route("/admin/model-prices", post(admin_prices::upsert_model_price).get(admin_prices::list_model_prices))
        .route("/admin/model-prices/{provider}/{model}", get(admin_prices::get_model_price))
        .route("/v1/token/balance", get(token_info::token_balance))
        .route("/v1/token/usage", get(token_info::token_usage))
}
