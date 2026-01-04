use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Channel {
    pub id: Uuid,
    pub server_id: Option<Uuid>, // None for DMs
    pub name: String,
    pub topic: Option<String>,
    pub channel_type: ChannelType,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "channel_type", rename_all = "snake_case")]
pub enum ChannelType {
    Text,
    Voice,
    DirectMessage,
    GroupDm,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DirectMessageChannel {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub user1_id: Uuid,
    pub user2_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GroupDmChannel {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub name: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GroupDmMember {
    pub id: Uuid,
    pub group_dm_id: Uuid,
    pub user_id: Uuid,
    pub joined_at: DateTime<Utc>,
}

/// Tracks who is currently in a voice channel
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VoiceState {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub user_id: Uuid,
    pub muted: bool,
    pub deafened: bool,
    pub self_muted: bool,
    pub self_deafened: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub topic: Option<String>,
    pub channel_type: ChannelType,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannel {
    pub name: Option<String>,
    pub topic: Option<String>,
}
