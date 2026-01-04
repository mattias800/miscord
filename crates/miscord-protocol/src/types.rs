use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User data shared between client and server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserData {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub custom_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    #[default]
    Offline,
    Online,
    Idle,
    DoNotDisturb,
    Invisible,
}

/// Server (guild) data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerData {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Channel data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelData {
    pub id: Uuid,
    pub server_id: Option<Uuid>,
    pub name: String,
    pub topic: Option<String>,
    pub channel_type: ChannelType,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    Text,
    Voice,
    DirectMessage,
    GroupDm,
}

/// Message data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Voice state data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceStateData {
    pub muted: bool,
    pub deafened: bool,
    pub self_muted: bool,
    pub self_deafened: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
}

/// ICE server configuration for WebRTC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Attachment data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentData {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
}

/// Reaction count
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionData {
    pub emoji: String,
    pub count: i64,
    pub reacted_by_me: bool,
}
