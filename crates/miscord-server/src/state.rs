use crate::services::{
    attachment::AttachmentService, channel::ChannelService, message::MessageService,
    user::UserService,
};
use crate::sfu::SfuSessionManager;
use crate::ws::connections::ConnectionManager;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct Config {
    pub bind_address: String,
    pub database_url: String,
    pub jwt_secret: String,
    pub stun_servers: Vec<String>,
    pub turn_servers: Vec<TurnServer>,
    pub upload_dir: PathBuf,
    pub base_url: String,
    pub tenor_api_key: Option<String>,
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

        let upload_dir = std::env::var("UPLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./uploads"));

        let base_url = std::env::var("BASE_URL")
            .unwrap_or_else(|_| format!("http://{}", bind_address));

        let tenor_api_key = std::env::var("TENOR_API_KEY").ok();
        if tenor_api_key.is_none() {
            tracing::info!("TENOR_API_KEY not set, GIF search will be disabled");
        }

        Ok(Config {
            bind_address,
            database_url,
            jwt_secret,
            stun_servers,
            turn_servers: vec![], // Configure via env if needed
            upload_dir,
            base_url,
            tenor_api_key,
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
    pub attachment_service: AttachmentService,
    pub sfu: Arc<SfuSessionManager>,
}

impl AppState {
    pub fn new(config: Config, db: PgPool) -> Self {
        let connections = Arc::new(ConnectionManager::new());
        let user_service = UserService::new(db.clone());
        let channel_service = ChannelService::new(db.clone());
        let message_service = MessageService::new(db.clone());
        let attachment_service = AttachmentService::new(
            db.clone(),
            config.upload_dir.clone(),
            config.base_url.clone(),
        );

        // Create SFU session manager with ICE servers from config
        let turn_servers: Vec<(String, String, String)> = config
            .turn_servers
            .iter()
            .map(|t| (t.url.clone(), t.username.clone(), t.credential.clone()))
            .collect();

        let sfu = SfuSessionManager::new(config.stun_servers.clone(), turn_servers)
            .expect("Failed to create SFU session manager");

        Self {
            config,
            db,
            connections,
            user_service,
            channel_service,
            message_service,
            attachment_service,
            sfu: Arc::new(sfu),
        }
    }
}
