mod config;
mod server;
mod routing;
mod providers;
mod logging;

use tracing_subscriber::fmt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    fmt::init();

    let config = config::Settings::load()?;

    // Use configured host/port to bind the server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let app = server::create_app(config).await?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Gateway server running on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
