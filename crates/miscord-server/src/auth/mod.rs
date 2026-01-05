use crate::error::{AppError, Result};
use crate::state::AppState;
use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::future::Future;
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

/// Authenticated user extractor
pub struct AuthUser {
    pub user_id: Uuid,
    pub username: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = std::result::Result<Self, Self::Rejection>> + Send {
        // Extract the Authorization header
        let auth_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let jwt_secret = state.config.jwt_secret.clone();

        async move {
            let auth_header = auth_header.ok_or(AppError::Unauthorized)?;

            // Extract the bearer token
            let token = auth_header
                .strip_prefix("Bearer ")
                .ok_or(AppError::Unauthorized)?;

            // Verify the token using the JWT secret from config
            let claims = verify_token(token, &jwt_secret)
                .map_err(|_| AppError::Unauthorized)?;

            Ok(AuthUser {
                user_id: claims.sub,
                username: claims.username,
            })
        }
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
