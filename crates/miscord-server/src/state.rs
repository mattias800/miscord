use crate::services::{channel::ChannelService, message::MessageService, user::UserService};
use crate::ws::connections::ConnectionManager;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Config {
    pub bind_address: String,
    pub database_url: String,
    pub jwt_secret: String,
    pub stun_servers: Vec<String>,
    pub turn_servers: Vec<TurnServer>,
}

#[derive(Clone)]
pub struct TurnServer {
    pub url: String,
    pub username: String,
    pub credential: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // Load from environment variables or config file
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:miscord.db".to_string());

        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
            tracing::warn!("JWT_SECRET not set, using default (insecure for production!)");
            "dev-secret-change-in-production".to_string()
        });

        let bind_address =
            std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

        let stun_servers = std::env::var("STUN_SERVERS")
            .map(|s| s.split(',').map(String::from).collect())
            .unwrap_or_else(|_| vec!["stun:stun.l.google.com:19302".to_string()]);

        Ok(Config {
            bind_address,
            database_url,
            jwt_secret,
            stun_servers,
            turn_servers: vec![], // Configure via env if needed
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub connections: Arc<ConnectionManager>,
    pub user_service: UserService,
    pub channel_service: ChannelService,
    pub message_service: MessageService,
}

impl AppState {
    pub fn new(config: Config, db: PgPool) -> Self {
        let connections = Arc::new(ConnectionManager::new());
        let user_service = UserService::new(db.clone());
        let channel_service = ChannelService::new(db.clone());
        let message_service = MessageService::new(db.clone());

        Self {
            config,
            db,
            connections,
            user_service,
            channel_service,
            message_service,
        }
    }
}
