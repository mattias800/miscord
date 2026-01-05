//! Miscord Server Library
//!
//! This module exposes the server components for testing and embedding.

pub mod api;
pub mod auth;
pub mod db;
pub mod error;
pub mod models;
pub mod services;
pub mod state;
pub mod webrtc;
pub mod ws;

use anyhow::Result;

/// Create and configure the server application
pub async fn create_app(config: state::Config) -> Result<(axum::Router, sqlx::PgPool)> {
    let db_pool = db::init_pool(&config.database_url).await?;
    db::run_migrations(&db_pool).await?;
    let app_state = state::AppState::new(config, db_pool.clone());
    let router = api::create_router(app_state);
    Ok((router, db_pool))
}
