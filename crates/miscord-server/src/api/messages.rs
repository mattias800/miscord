use crate::auth::AuthUser;
use crate::error::Result;
use crate::models::{CreateMessage, Message, UpdateMessage};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub before: Option<Uuid>,
    pub limit: Option<i64>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(channel_id): Path<Uuid>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<Message>>> {
    let limit = query.limit.unwrap_or(50).min(100);
    let messages = state
        .message_service
        .list_by_channel(channel_id, query.before, limit)
        .await?;
    Ok(Json(messages))
}

pub async fn create_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<Message>> {
    let message = state
        .message_service
        .create(channel_id, auth.user_id, input)
        .await?;

    // Broadcast to channel subscribers
    state.connections.broadcast_to_channel(
        channel_id,
        &miscord_protocol::ServerMessage::MessageCreated {
            message: miscord_protocol::MessageData {
                id: message.id,
                channel_id: message.channel_id,
                author_id: message.author_id,
                content: message.content.clone(),
                edited_at: message.edited_at,
                reply_to_id: message.reply_to_id,
                created_at: message.created_at,
            },
        },
    ).await;

    Ok(Json(message))
}

pub async fn update_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateMessage>,
) -> Result<Json<Message>> {
    let message = state
        .message_service
        .update(id, auth.user_id, input)
        .await?;

    // Broadcast update
    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::MessageUpdated {
            message: miscord_protocol::MessageData {
                id: message.id,
                channel_id: message.channel_id,
                author_id: message.author_id,
                content: message.content.clone(),
                edited_at: message.edited_at,
                reply_to_id: message.reply_to_id,
                created_at: message.created_at,
            },
        },
    ).await;

    Ok(Json(message))
}

pub async fn delete_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<()> {
    let message = state.message_service.get_by_id(id).await?;
    state.message_service.delete(id, auth.user_id).await?;

    // Broadcast deletion
    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::MessageDeleted {
            message_id: id,
            channel_id: message.channel_id,
        },
    ).await;

    Ok(())
}

pub async fn add_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, emoji)): Path<(Uuid, String)>,
) -> Result<()> {
    state
        .message_service
        .add_reaction(id, auth.user_id, &emoji)
        .await?;

    let message = state.message_service.get_by_id(id).await?;

    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::ReactionAdded {
            message_id: id,
            user_id: auth.user_id,
            emoji,
        },
    ).await;

    Ok(())
}

pub async fn remove_reaction(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, emoji)): Path<(Uuid, String)>,
) -> Result<()> {
    state
        .message_service
        .remove_reaction(id, auth.user_id, &emoji)
        .await?;

    let message = state.message_service.get_by_id(id).await?;

    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::ReactionRemoved {
            message_id: id,
            user_id: auth.user_id,
            emoji,
        },
    ).await;

    Ok(())
}
