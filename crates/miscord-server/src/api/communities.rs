use crate::auth::AuthUser;
use crate::error::{AppError, Result};
use crate::models::{Channel, ChannelType, Community, CommunityInvite, CreateChannel, CreateCommunity, PublicUser, UpdateCommunity, UserStatus};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use rand::Rng;
use uuid::Uuid;

pub async fn create_community(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<CreateCommunity>,
) -> Result<Json<Community>> {
    let community = sqlx::query_as!(
        Community,
        r#"
        INSERT INTO communities (id, name, description, owner_id, created_at, updated_at)
        VALUES ($1, $2, $3, $4, NOW(), NOW())
        RETURNING id, name, description, icon_url, owner_id, created_at, updated_at
        "#,
        Uuid::new_v4(),
        input.name,
        input.description,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?;

    // Add owner as a member
    sqlx::query!(
        r#"
        INSERT INTO community_members (id, community_id, user_id, joined_at)
        VALUES ($1, $2, $3, NOW())
        "#,
        Uuid::new_v4(),
        community.id,
        auth.user_id
    )
    .execute(&state.db)
    .await?;

    // Create default channels
    state
        .channel_service
        .create(
            community.id,
            CreateChannel {
                name: "general".to_string(),
                topic: Some("General discussion".to_string()),
                channel_type: crate::models::ChannelType::Text,
            },
        )
        .await?;

    state
        .channel_service
        .create(
            community.id,
            CreateChannel {
                name: "General".to_string(),
                topic: Some("Voice chat".to_string()),
                channel_type: crate::models::ChannelType::Voice,
            },
        )
        .await?;

    Ok(Json(community))
}

pub async fn list_communities(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Community>>> {
    let communities = sqlx::query_as!(
        Community,
        r#"
        SELECT c.id, c.name, c.description, c.icon_url, c.owner_id, c.created_at, c.updated_at
        FROM communities c
        INNER JOIN community_members m ON c.id = m.community_id
        WHERE m.user_id = $1
        ORDER BY c.name
        "#,
        auth.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(communities))
}

pub async fn get_community(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Community>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    let community = sqlx::query_as!(
        Community,
        r#"
        SELECT id, name, description, icon_url, owner_id, created_at, updated_at
        FROM communities WHERE id = $1
        "#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Community not found".to_string()))?;

    Ok(Json(community))
}

pub async fn update_community(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateCommunity>,
) -> Result<Json<Community>> {
    // Check ownership
    let community = sqlx::query_as!(
        Community,
        r#"
        SELECT id, name, description, icon_url, owner_id, created_at, updated_at
        FROM communities WHERE id = $1
        "#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Community not found".to_string()))?;

    if community.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    let updated = sqlx::query_as!(
        Community,
        r#"
        UPDATE communities
        SET name = COALESCE($2, name),
            description = COALESCE($3, description),
            icon_url = COALESCE($4, icon_url),
            updated_at = NOW()
        WHERE id = $1
        RETURNING id, name, description, icon_url, owner_id, created_at, updated_at
        "#,
        id,
        input.name,
        input.description,
        input.icon_url
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(updated))
}

pub async fn delete_community(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<()> {
    // Check ownership
    let community = sqlx::query_as!(
        Community,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM communities WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Community not found".to_string()))?;

    if community.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    sqlx::query!("DELETE FROM communities WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    Ok(())
}

pub async fn list_channels(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(community_id): Path<Uuid>,
) -> Result<Json<Vec<miscord_protocol::ChannelData>>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        community_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    let channels = state.channel_service.list_by_community(community_id).await?;

    // Get unread counts for all channels
    let channel_ids: Vec<Uuid> = channels.iter().map(|c| c.id).collect();
    let unread_counts = state
        .channel_service
        .get_unread_counts_for_channels(&channel_ids, auth.user_id)
        .await?;

    // Convert to protocol ChannelData with unread counts
    let channel_data: Vec<miscord_protocol::ChannelData> = channels
        .into_iter()
        .map(|c| {
            let unread_count = unread_counts.get(&c.id).copied().unwrap_or(0);
            miscord_protocol::ChannelData {
                id: c.id,
                community_id: c.community_id,
                name: c.name,
                topic: c.topic,
                channel_type: match c.channel_type {
                    ChannelType::Text => miscord_protocol::ChannelType::Text,
                    ChannelType::Voice => miscord_protocol::ChannelType::Voice,
                    ChannelType::DirectMessage => miscord_protocol::ChannelType::DirectMessage,
                    ChannelType::GroupDm => miscord_protocol::ChannelType::GroupDm,
                },
                position: c.position,
                unread_count,
            }
        })
        .collect();

    Ok(Json(channel_data))
}

pub async fn create_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(community_id): Path<Uuid>,
    Json(input): Json<CreateChannel>,
) -> Result<Json<Channel>> {
    // Check ownership (only owner can create channels for now)
    let community = sqlx::query_as!(
        Community,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM communities WHERE id = $1",
        community_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Community not found".to_string()))?;

    if community.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    let channel = state.channel_service.create(community_id, input).await?;
    Ok(Json(channel))
}

pub async fn create_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(community_id): Path<Uuid>,
) -> Result<Json<CommunityInvite>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        community_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    // Generate random invite code
    let code: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let invite = sqlx::query_as!(
        CommunityInvite,
        r#"
        INSERT INTO community_invites (id, community_id, code, created_by, uses, created_at)
        VALUES ($1, $2, $3, $4, 0, NOW())
        RETURNING id, community_id, code, created_by, uses, max_uses, expires_at, created_at
        "#,
        Uuid::new_v4(),
        community_id,
        code,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(invite))
}

pub async fn join_community(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(code): Path<String>,
) -> Result<Json<Community>> {
    // Find the invite
    let invite = sqlx::query_as!(
        CommunityInvite,
        r#"
        SELECT id, community_id, code, created_by, uses, max_uses, expires_at, created_at
        FROM community_invites WHERE code = $1
        "#,
        code
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Invalid invite code".to_string()))?;

    // Check if invite is valid
    if let Some(max_uses) = invite.max_uses {
        if invite.uses >= max_uses {
            return Err(AppError::BadRequest("Invite has expired".to_string()));
        }
    }

    if let Some(expires_at) = invite.expires_at {
        if expires_at < chrono::Utc::now() {
            return Err(AppError::BadRequest("Invite has expired".to_string()));
        }
    }

    // Check if already a member
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        invite.community_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        // Add as member
        sqlx::query!(
            "INSERT INTO community_members (id, community_id, user_id, joined_at) VALUES ($1, $2, $3, NOW())",
            Uuid::new_v4(),
            invite.community_id,
            auth.user_id
        )
        .execute(&state.db)
        .await?;

        // Increment uses
        sqlx::query!(
            "UPDATE community_invites SET uses = uses + 1 WHERE id = $1",
            invite.id
        )
        .execute(&state.db)
        .await?;
    }

    let community = sqlx::query_as!(
        Community,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM communities WHERE id = $1",
        invite.community_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(community))
}

pub async fn list_members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(community_id): Path<Uuid>,
) -> Result<Json<Vec<PublicUser>>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM community_members WHERE community_id = $1 AND user_id = $2)",
        community_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    // Get all members with their user data
    let members = sqlx::query_as!(
        PublicUser,
        r#"
        SELECT u.id, u.username, u.display_name, u.avatar_url,
               u.status as "status: UserStatus", u.custom_status
        FROM users u
        INNER JOIN community_members m ON u.id = m.user_id
        WHERE m.community_id = $1
        ORDER BY u.display_name
        "#,
        community_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(members))
}
