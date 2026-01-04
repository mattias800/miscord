use crate::auth::AuthUser;
use crate::error::Result;
use crate::models::{Channel, UpdateChannel, VoiceState};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

pub async fn get_channel(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Channel>> {
    let channel = state.channel_service.get_by_id(id).await?;
    Ok(Json(channel))
}

pub async fn update_channel(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateChannel>,
) -> Result<Json<Channel>> {
    let channel = state.channel_service.update(id, input).await?;
    Ok(Json(channel))
}

pub async fn delete_channel(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<()> {
    state.channel_service.delete(id).await?;
    Ok(())
}

// Voice channel endpoints

pub async fn join_voice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<Uuid>,
) -> Result<Json<VoiceState>> {
    let voice_state = state
        .channel_service
        .join_voice(channel_id, auth.user_id)
        .await?;

    // Notify other participants via WebSocket
    state.connections.broadcast_to_channel(
        channel_id,
        &miscord_protocol::ServerMessage::VoiceStateUpdate {
            channel_id,
            user_id: auth.user_id,
            state: miscord_protocol::VoiceStateData {
                muted: voice_state.muted,
                deafened: voice_state.deafened,
                self_muted: voice_state.self_muted,
                self_deafened: voice_state.self_deafened,
                video_enabled: voice_state.video_enabled,
                screen_sharing: voice_state.screen_sharing,
            },
        },
    ).await;

    Ok(Json(voice_state))
}

pub async fn leave_voice(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<()> {
    state.channel_service.leave_voice(auth.user_id).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoiceStateRequest {
    pub self_muted: Option<bool>,
    pub self_deafened: Option<bool>,
    pub video_enabled: Option<bool>,
    pub screen_sharing: Option<bool>,
}

pub async fn update_voice_state(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<UpdateVoiceStateRequest>,
) -> Result<Json<VoiceState>> {
    let voice_state = state
        .channel_service
        .update_voice_state(
            auth.user_id,
            input.self_muted,
            input.self_deafened,
            input.video_enabled,
            input.screen_sharing,
        )
        .await?;

    // Notify other participants
    state.connections.broadcast_to_channel(
        voice_state.channel_id,
        &miscord_protocol::ServerMessage::VoiceStateUpdate {
            channel_id: voice_state.channel_id,
            user_id: auth.user_id,
            state: miscord_protocol::VoiceStateData {
                muted: voice_state.muted,
                deafened: voice_state.deafened,
                self_muted: voice_state.self_muted,
                self_deafened: voice_state.self_deafened,
                video_enabled: voice_state.video_enabled,
                screen_sharing: voice_state.screen_sharing,
            },
        },
    ).await;

    Ok(Json(voice_state))
}

// Direct message endpoints

pub async fn list_dms(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Channel>>> {
    let channels = state.channel_service.get_user_dms(auth.user_id).await?;
    Ok(Json(channels))
}

pub async fn create_dm(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Channel>> {
    let channel = state
        .channel_service
        .get_or_create_dm(auth.user_id, user_id)
        .await?;
    Ok(Json(channel))
}
