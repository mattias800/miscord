use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: Uuid,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterResponse {
    pub user_id: Uuid,
    pub username: String,
}
