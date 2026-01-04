use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Message {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author_id: Uuid,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MessageAttachment {
    pub id: Uuid,
    pub message_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub url: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MessageReaction {
    pub id: Uuid,
    pub message_id: Uuid,
    pub user_id: Uuid,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessage {
    pub content: String,
    pub reply_to_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMessage {
    pub content: String,
}

/// Message with author information included
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithAuthor {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub author: MessageAuthor,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to_id: Option<Uuid>,
    pub attachments: Vec<MessageAttachment>,
    pub reactions: Vec<ReactionCount>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAuthor {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: i64,
    pub reacted_by_me: bool,
}
