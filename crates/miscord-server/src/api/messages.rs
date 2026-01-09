use crate::auth::AuthUser;
use crate::error::Result;
use crate::models::{CreateMessage, UpdateMessage};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use miscord_protocol::MessageData;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub before: Option<Uuid>,
    pub limit: Option<i64>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<Uuid>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<Vec<MessageData>>> {
    let limit = query.limit.unwrap_or(50).min(100);
    let messages = state
        .message_service
        .list_by_channel(channel_id, query.before, limit)
        .await?;

    // Get message IDs for batch reaction lookup
    let message_ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();

    // Get reactions for all messages in one query
    let reactions_map = state
        .message_service
        .get_reactions_for_messages(&message_ids, auth.user_id)
        .await
        .unwrap_or_default();

    // Convert to MessageData with author names and reactions
    let mut result = Vec::with_capacity(messages.len());
    for msg in messages {
        let author_name = state
            .user_service
            .get_by_id(msg.author_id)
            .await
            .map(|u| u.display_name)
            .unwrap_or_else(|_| "Unknown".to_string());

        // Get reactions for this message
        let reactions = reactions_map
            .get(&msg.id)
            .map(|r| {
                r.iter()
                    .map(|(emoji, count, reacted_by_me)| miscord_protocol::ReactionData {
                        emoji: emoji.clone(),
                        count: *count,
                        reacted_by_me: *reacted_by_me,
                    })
                    .collect()
            })
            .unwrap_or_default();

        result.push(MessageData {
            id: msg.id,
            channel_id: msg.channel_id,
            author_id: msg.author_id,
            author_name,
            content: msg.content,
            edited_at: msg.edited_at,
            reply_to_id: msg.reply_to_id,
            reactions,
            created_at: msg.created_at,
        });
    }

    Ok(Json(result))
}

pub async fn create_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<Uuid>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<MessageData>> {
    let message = state
        .message_service
        .create(channel_id, auth.user_id, input)
        .await?;

    // Get author name for the broadcast
    let author_name = state
        .user_service
        .get_by_id(auth.user_id)
        .await
        .map(|u| u.display_name)
        .unwrap_or_else(|_| "Unknown".to_string());

    let message_data = MessageData {
        id: message.id,
        channel_id: message.channel_id,
        author_id: message.author_id,
        author_name,
        content: message.content,
        edited_at: message.edited_at,
        reply_to_id: message.reply_to_id,
        reactions: vec![], // New messages have no reactions
        created_at: message.created_at,
    };

    // Broadcast to channel subscribers
    state.connections.broadcast_to_channel(
        channel_id,
        &miscord_protocol::ServerMessage::MessageCreated {
            message: message_data.clone(),
        },
    ).await;

    Ok(Json(message_data))
}

pub async fn update_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateMessage>,
) -> Result<Json<MessageData>> {
    let message = state
        .message_service
        .update(id, auth.user_id, input)
        .await?;

    // Get author name for the broadcast
    let author_name = state
        .user_service
        .get_by_id(message.author_id)
        .await
        .map(|u| u.display_name)
        .unwrap_or_else(|_| "Unknown".to_string());

    // Get reactions for the updated message
    let reactions = state
        .message_service
        .get_reactions(message.id, auth.user_id)
        .await
        .map(|r| {
            r.into_iter()
                .map(|(emoji, count, reacted_by_me)| miscord_protocol::ReactionData {
                    emoji,
                    count,
                    reacted_by_me,
                })
                .collect()
        })
        .unwrap_or_default();

    let message_data = MessageData {
        id: message.id,
        channel_id: message.channel_id,
        author_id: message.author_id,
        author_name,
        content: message.content,
        edited_at: message.edited_at,
        reply_to_id: message.reply_to_id,
        reactions,
        created_at: message.created_at,
    };

    // Broadcast update
    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::MessageUpdated {
            message: message_data.clone(),
        },
    ).await;

    Ok(Json(message_data))
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
