use crate::error::{AppError, Result};
use crate::models::{Channel, ChannelType, CreateChannel, UpdateChannel, VoiceState};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct ChannelService {
    db: PgPool,
}

impl ChannelService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create(&self, community_id: Uuid, input: CreateChannel) -> Result<Channel> {
        // Get the next position
        let max_position: Option<i32> = sqlx::query_scalar!(
            "SELECT MAX(position) FROM channels WHERE community_id = $1",
            community_id
        )
        .fetch_one(&self.db)
        .await?;

        let position = max_position.unwrap_or(0) + 1;

        let channel = sqlx::query_as!(
            Channel,
            r#"
            INSERT INTO channels (id, community_id, name, topic, channel_type, position, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
            RETURNING id, community_id, name, topic, channel_type as "channel_type: ChannelType",
                      position, created_at, updated_at
            "#,
            Uuid::new_v4(),
            community_id,
            input.name,
            input.topic,
            input.channel_type as ChannelType,
            position
        )
        .fetch_one(&self.db)
        .await?;

        Ok(channel)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Channel> {
        let channel = sqlx::query_as!(
            Channel,
            r#"
            SELECT id, community_id, name, topic, channel_type as "channel_type: ChannelType",
                   position, created_at, updated_at
            FROM channels WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".to_string()))?;

        Ok(channel)
    }

    pub async fn list_by_community(&self, community_id: Uuid) -> Result<Vec<Channel>> {
        let channels = sqlx::query_as!(
            Channel,
            r#"
            SELECT id, community_id, name, topic, channel_type as "channel_type: ChannelType",
                   position, created_at, updated_at
            FROM channels WHERE community_id = $1
            ORDER BY position
            "#,
            community_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(channels)
    }

    pub async fn update(&self, id: Uuid, input: UpdateChannel) -> Result<Channel> {
        let channel = sqlx::query_as!(
            Channel,
            r#"
            UPDATE channels
            SET name = COALESCE($2, name),
                topic = COALESCE($3, topic),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, community_id, name, topic, channel_type as "channel_type: ChannelType",
                      position, created_at, updated_at
            "#,
            id,
            input.name,
            input.topic
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".to_string()))?;

        Ok(channel)
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let result = sqlx::query!("DELETE FROM channels WHERE id = $1", id)
            .execute(&self.db)
            .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("Channel not found".to_string()));
        }

        Ok(())
    }

    // Voice channel operations

    pub async fn join_voice(&self, channel_id: Uuid, user_id: Uuid) -> Result<VoiceState> {
        // First leave any existing voice channel
        self.leave_voice(user_id).await?;

        let state = sqlx::query_as!(
            VoiceState,
            r#"
            INSERT INTO voice_states (id, channel_id, user_id, muted, deafened, self_muted,
                                      self_deafened, video_enabled, screen_sharing, joined_at)
            VALUES ($1, $2, $3, false, false, false, false, false, false, NOW())
            RETURNING id, channel_id, user_id, muted, deafened, self_muted, self_deafened,
                      video_enabled, screen_sharing, joined_at
            "#,
            Uuid::new_v4(),
            channel_id,
            user_id
        )
        .fetch_one(&self.db)
        .await?;

        Ok(state)
    }

    pub async fn leave_voice(&self, user_id: Uuid) -> Result<Option<Uuid>> {
        // Get the channel_id before deleting
        let result = sqlx::query!(
            "SELECT channel_id FROM voice_states WHERE user_id = $1",
            user_id
        )
        .fetch_optional(&self.db)
        .await?;

        let channel_id = result.map(|r| r.channel_id);

        sqlx::query!("DELETE FROM voice_states WHERE user_id = $1", user_id)
            .execute(&self.db)
            .await?;

        Ok(channel_id)
    }

    pub async fn get_voice_participants(&self, channel_id: Uuid) -> Result<Vec<VoiceState>> {
        let states = sqlx::query_as!(
            VoiceState,
            r#"
            SELECT id, channel_id, user_id, muted, deafened, self_muted, self_deafened,
                   video_enabled, screen_sharing, joined_at
            FROM voice_states WHERE channel_id = $1
            "#,
            channel_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(states)
    }

    pub async fn update_voice_state(
        &self,
        user_id: Uuid,
        self_muted: Option<bool>,
        self_deafened: Option<bool>,
        video_enabled: Option<bool>,
        screen_sharing: Option<bool>,
    ) -> Result<VoiceState> {
        let state = sqlx::query_as!(
            VoiceState,
            r#"
            UPDATE voice_states
            SET self_muted = COALESCE($2, self_muted),
                self_deafened = COALESCE($3, self_deafened),
                video_enabled = COALESCE($4, video_enabled),
                screen_sharing = COALESCE($5, screen_sharing)
            WHERE user_id = $1
            RETURNING id, channel_id, user_id, muted, deafened, self_muted, self_deafened,
                      video_enabled, screen_sharing, joined_at
            "#,
            user_id,
            self_muted,
            self_deafened,
            video_enabled,
            screen_sharing
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Not in a voice channel".to_string()))?;

        Ok(state)
    }

    // Direct message operations

    pub async fn get_or_create_dm(&self, user1_id: Uuid, user2_id: Uuid) -> Result<Channel> {
        // Check if DM already exists
        let existing = sqlx::query_as!(
            Channel,
            r#"
            SELECT c.id, c.community_id, c.name, c.topic, c.channel_type as "channel_type: ChannelType",
                   c.position, c.created_at, c.updated_at
            FROM channels c
            INNER JOIN direct_message_channels dm ON c.id = dm.channel_id
            WHERE (dm.user1_id = $1 AND dm.user2_id = $2) OR (dm.user1_id = $2 AND dm.user2_id = $1)
            "#,
            user1_id,
            user2_id
        )
        .fetch_optional(&self.db)
        .await?;

        if let Some(channel) = existing {
            return Ok(channel);
        }

        // Create new DM channel
        let channel_id = Uuid::new_v4();
        let channel = sqlx::query_as!(
            Channel,
            r#"
            INSERT INTO channels (id, community_id, name, topic, channel_type, position, created_at, updated_at)
            VALUES ($1, NULL, 'Direct Message', NULL, $2, 0, NOW(), NOW())
            RETURNING id, community_id, name, topic, channel_type as "channel_type: ChannelType",
                      position, created_at, updated_at
            "#,
            channel_id,
            ChannelType::DirectMessage as ChannelType
        )
        .fetch_one(&self.db)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO direct_message_channels (id, channel_id, user1_id, user2_id, created_at)
            VALUES ($1, $2, $3, $4, NOW())
            "#,
            Uuid::new_v4(),
            channel_id,
            user1_id,
            user2_id
        )
        .execute(&self.db)
        .await?;

        Ok(channel)
    }

    pub async fn get_user_dms(&self, user_id: Uuid) -> Result<Vec<Channel>> {
        let channels = sqlx::query_as!(
            Channel,
            r#"
            SELECT c.id, c.community_id, c.name, c.topic, c.channel_type as "channel_type: ChannelType",
                   c.position, c.created_at, c.updated_at
            FROM channels c
            INNER JOIN direct_message_channels dm ON c.id = dm.channel_id
            WHERE dm.user1_id = $1 OR dm.user2_id = $1
            ORDER BY c.updated_at DESC
            "#,
            user_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(channels)
    }

    // Channel read state operations

    /// Mark a channel as read for a user (upsert last_read_at to now)
    pub async fn mark_channel_read(&self, channel_id: Uuid, user_id: Uuid) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO channel_read_states (user_id, channel_id, last_read_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (user_id, channel_id)
            DO UPDATE SET last_read_at = NOW()
            "#,
            user_id,
            channel_id
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Get the last read timestamp for a user in a channel
    pub async fn get_last_read_at(
        &self,
        channel_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>> {
        let result = sqlx::query!(
            r#"
            SELECT last_read_at FROM channel_read_states
            WHERE user_id = $1 AND channel_id = $2
            "#,
            user_id,
            channel_id
        )
        .fetch_optional(&self.db)
        .await?;

        Ok(result.map(|r| r.last_read_at))
    }

    /// Get unread message count for a channel
    pub async fn get_unread_count(&self, channel_id: Uuid, user_id: Uuid) -> Result<i64> {
        let result = sqlx::query!(
            r#"
            SELECT COUNT(*) as count
            FROM messages m
            WHERE m.channel_id = $1
              AND m.thread_parent_id IS NULL
              AND m.created_at > COALESCE(
                  (SELECT last_read_at FROM channel_read_states
                   WHERE user_id = $2 AND channel_id = $1),
                  '1970-01-01'::timestamptz
              )
            "#,
            channel_id,
            user_id
        )
        .fetch_one(&self.db)
        .await?;

        Ok(result.count.unwrap_or(0))
    }

    /// Get unread counts for multiple channels at once
    pub async fn get_unread_counts_for_channels(
        &self,
        channel_ids: &[Uuid],
        user_id: Uuid,
    ) -> Result<std::collections::HashMap<Uuid, i64>> {
        if channel_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let results = sqlx::query!(
            r#"
            SELECT
                m.channel_id,
                COUNT(*) as count
            FROM messages m
            WHERE m.channel_id = ANY($1)
              AND m.thread_parent_id IS NULL
              AND m.created_at > COALESCE(
                  (SELECT last_read_at FROM channel_read_states crs
                   WHERE crs.user_id = $2 AND crs.channel_id = m.channel_id),
                  '1970-01-01'::timestamptz
              )
            GROUP BY m.channel_id
            "#,
            channel_ids,
            user_id
        )
        .fetch_all(&self.db)
        .await?;

        let mut counts = std::collections::HashMap::new();
        for row in results {
            counts.insert(row.channel_id, row.count.unwrap_or(0));
        }

        Ok(counts)
    }
}
