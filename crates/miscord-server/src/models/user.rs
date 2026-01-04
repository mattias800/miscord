use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub custom_status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, Default)]
#[sqlx(type_name = "user_status", rename_all = "lowercase")]
pub enum UserStatus {
    #[default]
    Offline,
    Online,
    Idle,
    DoNotDisturb,
    Invisible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: UserStatus,
    pub custom_status: Option<String>,
}

impl From<User> for PublicUser {
    fn from(user: User) -> Self {
        PublicUser {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
            status: user.status,
            custom_status: user.custom_status,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUser {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub custom_status: Option<String>,
}
