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

    // Get message IDs for batch reaction and attachment lookup
    let message_ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();

    // Get reactions for all messages in one query
    let reactions_map = state
        .message_service
        .get_reactions_for_messages(&message_ids, auth.user_id)
        .await
        .unwrap_or_default();

    // Get attachments for all messages in one query
    let attachments_list = state
        .attachment_service
        .get_by_message_ids(&message_ids)
        .await
        .unwrap_or_default();

    // Group attachments by message_id (skip orphan attachments without message_id)
    let mut attachments_map: std::collections::HashMap<Uuid, Vec<miscord_protocol::AttachmentData>> =
        std::collections::HashMap::new();
    for att in attachments_list {
        if let Some(message_id) = att.message_id {
            attachments_map
                .entry(message_id)
                .or_default()
                .push(miscord_protocol::AttachmentData {
                    id: att.id,
                    filename: att.filename,
                    content_type: att.content_type,
                    size_bytes: att.size_bytes,
                    url: att.url,
                });
        }
    }

    // Convert to MessageData with author names, reactions, and attachments
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
                    .map(|(emoji, user_ids, reacted_by_me)| miscord_protocol::ReactionData {
                        emoji: emoji.clone(),
                        user_ids: user_ids.clone(),
                        reacted_by_me: *reacted_by_me,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Get attachments for this message
        let attachments = attachments_map.remove(&msg.id).unwrap_or_default();

        result.push(MessageData {
            id: msg.id,
            channel_id: msg.channel_id,
            author_id: msg.author_id,
            author_name,
            content: msg.content,
            edited_at: msg.edited_at,
            reply_to_id: msg.reply_to_id,
            reactions,
            attachments,
            created_at: msg.created_at,
            thread_parent_id: msg.thread_parent_id,
            reply_count: msg.reply_count,
            last_reply_at: msg.last_reply_at,
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
    // Extract attachment_ids before passing to service
    let attachment_ids = input.attachment_ids.clone();

    let message = state
        .message_service
        .create(channel_id, auth.user_id, input)
        .await?;

    // Link attachments to the message if any were provided
    let attachments = if !attachment_ids.is_empty() {
        state
            .attachment_service
            .link_to_message(&attachment_ids, message.id)
            .await?;

        // Fetch the linked attachments
        state
            .attachment_service
            .get_by_message_id(message.id)
            .await
            .map(|atts| {
                atts.into_iter()
                    .map(|a| miscord_protocol::AttachmentData {
                        id: a.id,
                        filename: a.filename,
                        content_type: a.content_type,
                        size_bytes: a.size_bytes,
                        url: a.url,
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };

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
        attachments,
        created_at: message.created_at,
        thread_parent_id: message.thread_parent_id,
        reply_count: message.reply_count,
        last_reply_at: message.last_reply_at,
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
                .map(|(emoji, user_ids, reacted_by_me)| miscord_protocol::ReactionData {
                    emoji,
                    user_ids,
                    reacted_by_me,
                })
                .collect()
        })
        .unwrap_or_default();

    // Get attachments for the message
    let attachments = state
        .attachment_service
        .get_by_message_id(message.id)
        .await
        .map(|atts| {
            atts.into_iter()
                .map(|a| miscord_protocol::AttachmentData {
                    id: a.id,
                    filename: a.filename,
                    content_type: a.content_type,
                    size_bytes: a.size_bytes,
                    url: a.url,
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
        attachments,
        created_at: message.created_at,
        thread_parent_id: message.thread_parent_id,
        reply_count: message.reply_count,
        last_reply_at: message.last_reply_at,
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
    let thread_parent_id = state.message_service.delete(id, auth.user_id).await?;

    // Broadcast deletion
    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::MessageDeleted {
            message_id: id,
            channel_id: message.channel_id,
        },
    ).await;

    // If this was a thread reply, broadcast updated metadata for the parent
    if let Some(parent_id) = thread_parent_id {
        if let Ok(parent) = state.message_service.get_by_id(parent_id).await {
            state.connections.broadcast_to_channel(
                message.channel_id,
                &miscord_protocol::ServerMessage::ThreadMetadataUpdated {
                    message_id: parent_id,
                    reply_count: parent.reply_count,
                    last_reply_at: parent.last_reply_at,
                },
            ).await;
        }
    }

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

// Thread endpoints

/// Get thread with parent message and replies
pub async fn get_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(parent_id): Path<Uuid>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<miscord_protocol::ThreadData>> {
    let limit = query.limit.unwrap_or(50).min(100);

    // Get parent message
    let parent = state.message_service.get_by_id(parent_id).await?;
    let parent_author = state
        .user_service
        .get_by_id(parent.author_id)
        .await
        .map(|u| u.display_name)
        .unwrap_or_else(|_| "Unknown".to_string());

    // Get thread replies
    let replies = state
        .message_service
        .get_thread_replies(parent_id, limit)
        .await?;

    // Get all message IDs for batch reaction and attachment lookup
    let mut all_message_ids: Vec<Uuid> = replies.iter().map(|m| m.id).collect();
    all_message_ids.push(parent_id);

    let reactions_map = state
        .message_service
        .get_reactions_for_messages(&all_message_ids, auth.user_id)
        .await
        .unwrap_or_default();

    // Get attachments for all messages
    let attachments_list = state
        .attachment_service
        .get_by_message_ids(&all_message_ids)
        .await
        .unwrap_or_default();

    let mut attachments_map: std::collections::HashMap<Uuid, Vec<miscord_protocol::AttachmentData>> =
        std::collections::HashMap::new();
    for att in attachments_list {
        if let Some(message_id) = att.message_id {
            attachments_map
                .entry(message_id)
                .or_default()
                .push(miscord_protocol::AttachmentData {
                    id: att.id,
                    filename: att.filename,
                    content_type: att.content_type,
                    size_bytes: att.size_bytes,
                    url: att.url,
                });
        }
    }

    // Build parent MessageData
    let parent_reactions = reactions_map
        .get(&parent.id)
        .map(|r| {
            r.iter()
                .map(|(emoji, user_ids, reacted_by_me)| miscord_protocol::ReactionData {
                    emoji: emoji.clone(),
                    user_ids: user_ids.clone(),
                    reacted_by_me: *reacted_by_me,
                })
                .collect()
        })
        .unwrap_or_default();

    let parent_attachments = attachments_map.remove(&parent.id).unwrap_or_default();

    let parent_data = MessageData {
        id: parent.id,
        channel_id: parent.channel_id,
        author_id: parent.author_id,
        author_name: parent_author,
        content: parent.content,
        edited_at: parent.edited_at,
        reply_to_id: parent.reply_to_id,
        reactions: parent_reactions,
        attachments: parent_attachments,
        created_at: parent.created_at,
        thread_parent_id: parent.thread_parent_id,
        reply_count: parent.reply_count,
        last_reply_at: parent.last_reply_at,
    };

    // Build reply MessageData list
    let mut replies_data = Vec::with_capacity(replies.len());
    for msg in replies {
        let author_name = state
            .user_service
            .get_by_id(msg.author_id)
            .await
            .map(|u| u.display_name)
            .unwrap_or_else(|_| "Unknown".to_string());

        let reactions = reactions_map
            .get(&msg.id)
            .map(|r| {
                r.iter()
                    .map(|(emoji, user_ids, reacted_by_me)| miscord_protocol::ReactionData {
                        emoji: emoji.clone(),
                        user_ids: user_ids.clone(),
                        reacted_by_me: *reacted_by_me,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let attachments = attachments_map.remove(&msg.id).unwrap_or_default();

        replies_data.push(MessageData {
            id: msg.id,
            channel_id: msg.channel_id,
            author_id: msg.author_id,
            author_name,
            content: msg.content,
            edited_at: msg.edited_at,
            reply_to_id: msg.reply_to_id,
            reactions,
            attachments,
            created_at: msg.created_at,
            thread_parent_id: msg.thread_parent_id,
            reply_count: msg.reply_count,
            last_reply_at: msg.last_reply_at,
        });
    }

    Ok(Json(miscord_protocol::ThreadData {
        parent_message: parent_data,
        replies: replies_data,
        total_reply_count: parent.reply_count,
    }))
}

/// Create a thread reply
pub async fn create_thread_reply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(parent_id): Path<Uuid>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<MessageData>> {
    let message = state
        .message_service
        .create_thread_reply(parent_id, auth.user_id, input.content, input.reply_to_id)
        .await?;

    // Get author name
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
        reactions: vec![],
        attachments: vec![], // TODO: Support attachments on thread reply creation
        created_at: message.created_at,
        thread_parent_id: message.thread_parent_id,
        reply_count: message.reply_count,
        last_reply_at: message.last_reply_at,
    };

    // Get updated parent for metadata
    let parent = state.message_service.get_by_id(parent_id).await?;

    // Broadcast the new reply to thread subscribers
    state.connections.broadcast_to_thread(
        parent_id,
        &miscord_protocol::ServerMessage::ThreadReplyCreated {
            parent_message_id: parent_id,
            message: message_data.clone(),
        },
    ).await;

    // Broadcast updated parent metadata to channel subscribers
    state.connections.broadcast_to_channel(
        message.channel_id,
        &miscord_protocol::ServerMessage::ThreadMetadataUpdated {
            message_id: parent_id,
            reply_count: parent.reply_count,
            last_reply_at: parent.last_reply_at,
        },
    ).await;

    Ok(Json(message_data))
}

// Search endpoints

#[derive(Debug, Deserialize)]
pub struct SearchMessagesQuery {
    pub q: String,
    pub community_id: Option<Uuid>,
    pub limit: Option<i64>,
}

/// Search result with channel information
#[derive(Debug, serde::Serialize)]
pub struct MessageSearchResult {
    pub message: MessageData,
    pub channel_name: String,
    pub community_name: String,
}

/// Search messages by content
pub async fn search_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<Vec<MessageSearchResult>>> {
    let limit = query.limit.unwrap_or(20).min(50);

    // Don't search empty queries
    if query.q.trim().is_empty() {
        return Ok(Json(vec![]));
    }

    let messages = state
        .message_service
        .search_messages(&query.q, query.community_id, limit)
        .await?;

    if messages.is_empty() {
        return Ok(Json(vec![]));
    }

    // Get message IDs for batch lookups
    let message_ids: Vec<Uuid> = messages.iter().map(|m| m.id).collect();

    // Get reactions for all messages
    let reactions_map = state
        .message_service
        .get_reactions_for_messages(&message_ids, auth.user_id)
        .await
        .unwrap_or_default();

    // Get attachments for all messages
    let attachments_list = state
        .attachment_service
        .get_by_message_ids(&message_ids)
        .await
        .unwrap_or_default();

    // Group attachments by message_id
    let mut attachments_map: std::collections::HashMap<Uuid, Vec<miscord_protocol::AttachmentData>> =
        std::collections::HashMap::new();
    for att in attachments_list {
        if let Some(message_id) = att.message_id {
            attachments_map
                .entry(message_id)
                .or_default()
                .push(miscord_protocol::AttachmentData {
                    id: att.id,
                    filename: att.filename,
                    content_type: att.content_type,
                    size_bytes: att.size_bytes,
                    url: att.url,
                });
        }
    }

    // Build results with channel and community names
    let mut results = Vec::with_capacity(messages.len());
    for msg in messages {
        // Get author name
        let author_name = state
            .user_service
            .get_by_id(msg.author_id)
            .await
            .map(|u| u.display_name)
            .unwrap_or_else(|_| "Unknown".to_string());

        // Get channel info (use channel service)
        let channel_info = state.channel_service.get_by_id(msg.channel_id).await.ok();
        let channel_name = channel_info.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| "Unknown".to_string());
        let community_name = if let Some(ref ch) = channel_info {
            if let Some(comm_id) = ch.community_id {
                // Query community name directly
                sqlx::query_scalar!("SELECT name FROM communities WHERE id = $1", comm_id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "Unknown".to_string())
            } else {
                "Unknown".to_string()
            }
        } else {
            "Unknown".to_string()
        };

        // Get reactions for this message
        let reactions = reactions_map
            .get(&msg.id)
            .map(|r| {
                r.iter()
                    .map(|(emoji, user_ids, reacted_by_me)| miscord_protocol::ReactionData {
                        emoji: emoji.clone(),
                        user_ids: user_ids.clone(),
                        reacted_by_me: *reacted_by_me,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Get attachments for this message
        let attachments = attachments_map.remove(&msg.id).unwrap_or_default();

        let message_data = MessageData {
            id: msg.id,
            channel_id: msg.channel_id,
            author_id: msg.author_id,
            author_name,
            content: msg.content,
            edited_at: msg.edited_at,
            reply_to_id: msg.reply_to_id,
            reactions,
            attachments,
            created_at: msg.created_at,
            thread_parent_id: msg.thread_parent_id,
            reply_count: msg.reply_count,
            last_reply_at: msg.last_reply_at,
        };

        results.push(MessageSearchResult {
            message: message_data,
            channel_name,
            community_name,
        });
    }

    Ok(Json(results))
}
