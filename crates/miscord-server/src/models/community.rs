use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A Community - a group with channels and members
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Community {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CommunityMember {
    pub id: Uuid,
    pub community_id: Uuid,
    pub user_id: Uuid,
    pub nickname: Option<String>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CommunityRole {
    pub id: Uuid,
    pub community_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub permissions: i64,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommunity {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCommunity {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
}

/// Invite link to join a community
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CommunityInvite {
    pub id: Uuid,
    pub community_id: Uuid,
    pub code: String,
    pub created_by: Uuid,
    pub uses: i32,
    pub max_uses: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
