use crate::auth::AuthUser;
use crate::error::{AppError, Result};
use crate::models::{Channel, CreateChannel, CreateServer, Server, ServerInvite, UpdateServer};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use rand::Rng;
use uuid::Uuid;

pub async fn create_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<CreateServer>,
) -> Result<Json<Server>> {
    let server = sqlx::query_as!(
        Server,
        r#"
        INSERT INTO servers (id, name, description, owner_id, created_at, updated_at)
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
        INSERT INTO server_members (id, server_id, user_id, joined_at)
        VALUES ($1, $2, $3, NOW())
        "#,
        Uuid::new_v4(),
        server.id,
        auth.user_id
    )
    .execute(&state.db)
    .await?;

    // Create default channels
    state
        .channel_service
        .create(
            server.id,
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
            server.id,
            CreateChannel {
                name: "General".to_string(),
                topic: Some("Voice chat".to_string()),
                channel_type: crate::models::ChannelType::Voice,
            },
        )
        .await?;

    Ok(Json(server))
}

pub async fn list_servers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<Server>>> {
    let servers = sqlx::query_as!(
        Server,
        r#"
        SELECT s.id, s.name, s.description, s.icon_url, s.owner_id, s.created_at, s.updated_at
        FROM servers s
        INNER JOIN server_members m ON s.id = m.server_id
        WHERE m.user_id = $1
        ORDER BY s.name
        "#,
        auth.user_id
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(servers))
}

pub async fn get_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Server>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
        id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    let server = sqlx::query_as!(
        Server,
        r#"
        SELECT id, name, description, icon_url, owner_id, created_at, updated_at
        FROM servers WHERE id = $1
        "#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Server not found".to_string()))?;

    Ok(Json(server))
}

pub async fn update_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateServer>,
) -> Result<Json<Server>> {
    // Check ownership
    let server = sqlx::query_as!(
        Server,
        r#"
        SELECT id, name, description, icon_url, owner_id, created_at, updated_at
        FROM servers WHERE id = $1
        "#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Server not found".to_string()))?;

    if server.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    let updated = sqlx::query_as!(
        Server,
        r#"
        UPDATE servers
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

pub async fn delete_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<()> {
    // Check ownership
    let server = sqlx::query_as!(
        Server,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM servers WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Server not found".to_string()))?;

    if server.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    sqlx::query!("DELETE FROM servers WHERE id = $1", id)
        .execute(&state.db)
        .await?;

    Ok(())
}

pub async fn list_channels(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<Uuid>,
) -> Result<Json<Vec<Channel>>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
        server_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        return Err(AppError::Forbidden);
    }

    let channels = state.channel_service.list_by_server(server_id).await?;
    Ok(Json(channels))
}

pub async fn create_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<Uuid>,
    Json(input): Json<CreateChannel>,
) -> Result<Json<Channel>> {
    // Check ownership (only owner can create channels for now)
    let server = sqlx::query_as!(
        Server,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM servers WHERE id = $1",
        server_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Server not found".to_string()))?;

    if server.owner_id != auth.user_id {
        return Err(AppError::Forbidden);
    }

    let channel = state.channel_service.create(server_id, input).await?;
    Ok(Json(channel))
}

pub async fn create_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(server_id): Path<Uuid>,
) -> Result<Json<ServerInvite>> {
    // Check membership
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
        server_id,
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
        ServerInvite,
        r#"
        INSERT INTO server_invites (id, server_id, code, created_by, uses, created_at)
        VALUES ($1, $2, $3, $4, 0, NOW())
        RETURNING id, server_id, code, created_by, uses, max_uses, expires_at, created_at
        "#,
        Uuid::new_v4(),
        server_id,
        code,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(invite))
}

pub async fn join_server(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(code): Path<String>,
) -> Result<Json<Server>> {
    // Find the invite
    let invite = sqlx::query_as!(
        ServerInvite,
        r#"
        SELECT id, server_id, code, created_by, uses, max_uses, expires_at, created_at
        FROM server_invites WHERE code = $1
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
        "SELECT EXISTS(SELECT 1 FROM server_members WHERE server_id = $1 AND user_id = $2)",
        invite.server_id,
        auth.user_id
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(false);

    if !is_member {
        // Add as member
        sqlx::query!(
            "INSERT INTO server_members (id, server_id, user_id, joined_at) VALUES ($1, $2, $3, NOW())",
            Uuid::new_v4(),
            invite.server_id,
            auth.user_id
        )
        .execute(&state.db)
        .await?;

        // Increment uses
        sqlx::query!(
            "UPDATE server_invites SET uses = uses + 1 WHERE id = $1",
            invite.id
        )
        .execute(&state.db)
        .await?;
    }

    let server = sqlx::query_as!(
        Server,
        "SELECT id, name, description, icon_url, owner_id, created_at, updated_at FROM servers WHERE id = $1",
        invite.server_id
    )
    .fetch_one(&state.db)
    .await?;

    Ok(Json(server))
}
