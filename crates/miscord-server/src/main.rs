use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod auth;
mod db;
mod error;
mod models;
mod services;
mod state;
mod webrtc;
mod ws;

#[tokio::main]
async fn main() -> Result<()> {
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

    // Initialize database
    let db_pool = db::init_pool(&config.database_url).await?;
    db::run_migrations(&db_pool).await?;

    // Create application state
    let state = state::AppState::new(config.clone(), db_pool);

    // Build the router
    let app = api::create_router(state);

    // Start the server
    let listener = tokio::net::TcpListener::bind(&config.bind_address).await?;
    tracing::info!("Listening on {}", config.bind_address);

    axum::serve(listener, app).await?;

    Ok(())
}
