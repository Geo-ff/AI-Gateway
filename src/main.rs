mod admin;
mod balance;
mod config;
mod crypto;
mod db;
mod error;
mod logging;
mod password_reset_tokens;
mod providers;
mod refresh_tokens;
mod routing;
mod server;
mod subscription;
mod users;

use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> crate::error::Result<()> {
    // Local development: load `.env` without panicking (no-op if missing).
    dotenvy::dotenv().ok();

    // 使用自定义北京时间格式与环境过滤器
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_timer(crate::logging::time::BeijingTimer)
        .init();

    let config = config::Settings::load()?;

    // Use configured host/port to bind the server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let app = server::create_app(config).await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Gateway server running on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
