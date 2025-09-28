mod config;
mod server;
mod routing;
mod providers;
mod logging;
mod error;
mod crypto;
mod admin;
mod db;

use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> crate::error::Result<()> {
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
