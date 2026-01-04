use crate::error::{AppError, Result};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
    RequestPartsExt,
};
use axum_extra::{headers, TypedHeader};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,       // User ID
    pub username: String,
    pub exp: i64,        // Expiration time
    pub iat: i64,        // Issued at
}

impl Claims {
    pub fn new(user_id: Uuid, username: String, expires_in_hours: i64) -> Self {
        let now = Utc::now();
        Self {
            sub: user_id,
            username,
            exp: (now + Duration::hours(expires_in_hours)).timestamp(),
            iat: now.timestamp(),
        }
    }
}

pub fn create_token(user_id: Uuid, username: &str, secret: &str) -> Result<String> {
    let claims = Claims::new(user_id, username.to_string(), 24 * 7); // 7 days

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;

    Ok(token_data.claims)
}

/// Extractor for authenticated requests
pub struct AuthUser {
    pub user_id: Uuid,
    pub username: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> std::result::Result<Self, Self::Rejection> {
        // Get the authorization header
        let TypedHeader(auth_header) = parts
            .extract::<TypedHeader<headers::Authorization<headers::authorization::Bearer>>>()
            .await
            .map_err(|_| AppError::Unauthorized)?;

        let token = auth_header.token();

        // Get JWT secret from state - for now we'll use the env var directly
        // In production, this should come from app state
        let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-in-production".to_string());

        let claims = verify_token(token, &secret)?;

        Ok(AuthUser {
            user_id: claims.sub,
            username: claims.username,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: Uuid,
    pub username: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: Uuid,
    pub username: String,
}
