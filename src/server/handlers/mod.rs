use axum::{routing::{get, post}, Router};
use std::sync::Arc;

use crate::server::AppState;

mod chat;
mod models;
mod cache;
mod provider_keys;
mod providers;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
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
}
