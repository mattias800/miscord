use anyhow::Result;
use miscord_server::state;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "miscord_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Miscord server...");

    // Load configuration
    let config = state::Config::load()?;
    let bind_address = config.bind_address.clone();

    // Create application
    let (app, _db_pool) = miscord_server::create_app(config).await?;

    // Start the server
    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    tracing::info!("Listening on {}", bind_address);

    axum::serve(listener, app).await?;

    Ok(())
}
