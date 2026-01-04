use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A "Server" in Discord terminology - a community with channels
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Server {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ServerMember {
    pub id: Uuid,
    pub server_id: Uuid,
    pub user_id: Uuid,
    pub nickname: Option<String>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ServerRole {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub permissions: i64,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServer {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateServer {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
}

/// Invite link to join a server
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ServerInvite {
    pub id: Uuid,
    pub server_id: Uuid,
    pub code: String,
    pub created_by: Uuid,
    pub uses: i32,
    pub max_uses: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
