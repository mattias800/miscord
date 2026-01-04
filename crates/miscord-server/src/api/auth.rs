use crate::auth::{create_token, LoginRequest, LoginResponse, RegisterResponse};
use crate::error::Result;
use crate::models::CreateUser;
use crate::state::AppState;
use axum::{extract::State, Json};

pub async fn register(
    State(state): State<AppState>,
    Json(input): Json<CreateUser>,
) -> Result<Json<RegisterResponse>> {
    let user = state.user_service.create(input).await?;

    Ok(Json(RegisterResponse {
        user_id: user.id,
        username: user.username,
    }))
}

pub async fn login(
    State(state): State<AppState>,
    Json(input): Json<LoginRequest>,
) -> Result<Json<LoginResponse>> {
    let user = state
        .user_service
        .verify_credentials(&input.username, &input.password)
        .await?;

    let token = create_token(user.id, &user.username, &state.config.jwt_secret)?;

    Ok(Json(LoginResponse {
        token,
        user_id: user.id,
        username: user.username,
    }))
}
