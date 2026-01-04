use crate::auth::AuthUser;
use crate::error::Result;
use crate::models::{PublicUser, UpdateUser};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

pub async fn get_me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<PublicUser>> {
    let user = state.user_service.get_by_id(auth.user_id).await?;
    Ok(Json(user.into()))
}

pub async fn update_me(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<UpdateUser>,
) -> Result<Json<PublicUser>> {
    let user = state.user_service.update(auth.user_id, input).await?;
    Ok(Json(user.into()))
}

pub async fn get_user(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<PublicUser>> {
    let user = state.user_service.get_by_id(id).await?;
    Ok(Json(user.into()))
}

pub async fn get_friends(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PublicUser>>> {
    let friends = state.user_service.get_friends(auth.user_id).await?;
    Ok(Json(friends))
}
