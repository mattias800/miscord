use crate::error::{AppError, Result};
use crate::models::{CreateMessage, Message, MessageAttachment, UpdateMessage};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct MessageService {
    db: PgPool,
}

impl MessageService {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    pub async fn create(&self, channel_id: Uuid, author_id: Uuid, input: CreateMessage) -> Result<Message> {
        let message = sqlx::query_as!(
            Message,
            r#"
            INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, thread_parent_id, reply_count, created_at)
            VALUES ($1, $2, $3, $4, $5, NULL, 0, NOW())
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            "#,
            Uuid::new_v4(),
            channel_id,
            author_id,
            input.content,
            input.reply_to_id
        )
        .fetch_one(&self.db)
        .await?;

        // Update channel's updated_at timestamp
        sqlx::query!("UPDATE channels SET updated_at = NOW() WHERE id = $1", channel_id)
            .execute(&self.db)
            .await?;

        Ok(message)
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Message> {
        let message = sqlx::query_as!(
            Message,
            r#"
            SELECT id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            FROM messages WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

        Ok(message)
    }

    pub async fn list_by_channel(
        &self,
        channel_id: Uuid,
        before: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Message>> {
        let messages = if let Some(before_id) = before {
            // Get the created_at of the "before" message
            let before_msg = self.get_by_id(before_id).await?;

            sqlx::query_as!(
                Message,
                r#"
                SELECT id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
                FROM messages
                WHERE channel_id = $1 AND created_at < $2 AND thread_parent_id IS NULL
                ORDER BY created_at DESC
                LIMIT $3
                "#,
                channel_id,
                before_msg.created_at,
                limit
            )
            .fetch_all(&self.db)
            .await?
        } else {
            sqlx::query_as!(
                Message,
                r#"
                SELECT id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
                FROM messages
                WHERE channel_id = $1 AND thread_parent_id IS NULL
                ORDER BY created_at DESC
                LIMIT $2
                "#,
                channel_id,
                limit
            )
            .fetch_all(&self.db)
            .await?
        };

        Ok(messages)
    }

    pub async fn update(&self, id: Uuid, author_id: Uuid, input: UpdateMessage) -> Result<Message> {
        let message = sqlx::query_as!(
            Message,
            r#"
            UPDATE messages
            SET content = $3, edited_at = NOW()
            WHERE id = $1 AND author_id = $2
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            "#,
            id,
            author_id,
            input.content
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Message not found or not owned by user".to_string()))?;

        Ok(message)
    }

    /// Delete a message. Returns the thread_parent_id if this was a thread reply.
    pub async fn delete(&self, id: Uuid, author_id: Uuid) -> Result<Option<Uuid>> {
        // Get the message first to check if it's a thread reply
        let message = self.get_by_id(id).await?;

        // Verify ownership
        if message.author_id != author_id {
            return Err(AppError::NotFound(
                "Message not found or not owned by user".to_string(),
            ));
        }

        let thread_parent_id = message.thread_parent_id;

        let result = sqlx::query!(
            "DELETE FROM messages WHERE id = $1 AND author_id = $2",
            id,
            author_id
        )
        .execute(&self.db)
        .await?;

        if result.rows_affected() == 0 {
            return Err(AppError::NotFound(
                "Message not found or not owned by user".to_string(),
            ));
        }

        // If this was a thread reply, decrement parent's reply_count
        if let Some(parent_id) = thread_parent_id {
            sqlx::query!(
                "UPDATE messages SET reply_count = GREATEST(reply_count - 1, 0) WHERE id = $1",
                parent_id
            )
            .execute(&self.db)
            .await?;
        }

        Ok(thread_parent_id)
    }

    /// Create a thread reply
    pub async fn create_thread_reply(
        &self,
        parent_message_id: Uuid,
        author_id: Uuid,
        content: String,
        reply_to_id: Option<Uuid>,
    ) -> Result<Message> {
        // Get parent message to verify it exists and get channel_id
        let parent = self.get_by_id(parent_message_id).await?;

        // Create the reply with thread_parent_id set
        let message = sqlx::query_as!(
            Message,
            r#"
            INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, thread_parent_id, reply_count, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, 0, NOW())
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            "#,
            Uuid::new_v4(),
            parent.channel_id,
            author_id,
            content,
            reply_to_id,
            parent_message_id
        )
        .fetch_one(&self.db)
        .await?;

        // Update parent's reply_count and last_reply_at
        sqlx::query!(
            r#"
            UPDATE messages
            SET reply_count = reply_count + 1, last_reply_at = NOW()
            WHERE id = $1
            "#,
            parent_message_id
        )
        .execute(&self.db)
        .await?;

        Ok(message)
    }

    /// Get thread replies for a parent message
    pub async fn get_thread_replies(
        &self,
        parent_message_id: Uuid,
        limit: i64,
    ) -> Result<Vec<Message>> {
        let messages = sqlx::query_as!(
            Message,
            r#"
            SELECT id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            FROM messages
            WHERE thread_parent_id = $1
            ORDER BY created_at ASC
            LIMIT $2
            "#,
            parent_message_id,
            limit
        )
        .fetch_all(&self.db)
        .await?;

        Ok(messages)
    }

    pub async fn add_reaction(&self, message_id: Uuid, user_id: Uuid, emoji: &str) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO message_reactions (id, message_id, user_id, emoji, created_at)
            VALUES ($1, $2, $3, $4, NOW())
            ON CONFLICT (message_id, user_id, emoji) DO NOTHING
            "#,
            Uuid::new_v4(),
            message_id,
            user_id,
            emoji
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    pub async fn remove_reaction(&self, message_id: Uuid, user_id: Uuid, emoji: &str) -> Result<()> {
        sqlx::query!(
            "DELETE FROM message_reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3",
            message_id,
            user_id,
            emoji
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    pub async fn get_attachments(&self, message_id: Uuid) -> Result<Vec<MessageAttachment>> {
        let attachments = sqlx::query_as!(
            MessageAttachment,
            r#"
            SELECT id, message_id, filename, content_type, size_bytes, url, created_at
            FROM message_attachments WHERE message_id = $1
            "#,
            message_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(attachments)
    }

    /// Get reactions for a message with user IDs
    pub async fn get_reactions(&self, message_id: Uuid, user_id: Uuid) -> Result<Vec<(String, Vec<Uuid>, bool)>> {
        // Get all reactions with user IDs
        let reactions = sqlx::query!(
            r#"
            SELECT user_id, emoji
            FROM message_reactions
            WHERE message_id = $1
            ORDER BY emoji, created_at
            "#,
            message_id
        )
        .fetch_all(&self.db)
        .await?;

        // Group by emoji
        let mut emoji_users: std::collections::HashMap<String, Vec<Uuid>> = std::collections::HashMap::new();
        for r in reactions {
            emoji_users
                .entry(r.emoji)
                .or_default()
                .push(r.user_id);
        }

        // Convert to result format
        let mut result: Vec<(String, Vec<Uuid>, bool)> = emoji_users
            .into_iter()
            .map(|(emoji, user_ids)| {
                let reacted_by_me = user_ids.contains(&user_id);
                (emoji, user_ids, reacted_by_me)
            })
            .collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(result)
    }

    /// Search messages by content across channels the user has access to
    /// - For community channels: only searches in communities user is a member of
    /// - For DMs: only searches in DMs where user is a participant
    /// Returns messages matching the query, newest first
    pub async fn search_messages(
        &self,
        query: &str,
        user_id: Uuid,
        community_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Message>> {
        let search_pattern = format!("%{}%", query);

        let messages = if let Some(comm_id) = community_id {
            // Search within a specific community (verify membership)
            sqlx::query_as!(
                Message,
                r#"
                SELECT m.id, m.channel_id, m.author_id, m.content, m.edited_at, m.reply_to_id,
                       m.thread_parent_id, m.reply_count, m.last_reply_at, m.pinned_at, m.pinned_by_id, m.created_at
                FROM messages m
                JOIN channels c ON m.channel_id = c.id
                JOIN community_members cm ON c.community_id = cm.community_id
                WHERE m.content ILIKE $1
                  AND c.community_id = $2
                  AND cm.user_id = $3
                  AND m.thread_parent_id IS NULL
                ORDER BY m.created_at DESC
                LIMIT $4
                "#,
                search_pattern,
                comm_id,
                user_id,
                limit
            )
            .fetch_all(&self.db)
            .await?
        } else {
            // Search across all accessible channels:
            // 1. Community channels where user is a member
            // 2. DM channels where user is a participant
            sqlx::query_as!(
                Message,
                r#"
                SELECT m.id, m.channel_id, m.author_id, m.content, m.edited_at, m.reply_to_id,
                       m.thread_parent_id, m.reply_count, m.last_reply_at, m.pinned_at, m.pinned_by_id, m.created_at
                FROM messages m
                JOIN channels c ON m.channel_id = c.id
                WHERE m.content ILIKE $1
                  AND m.thread_parent_id IS NULL
                  AND (
                    -- Community channels: user must be a member
                    (c.community_id IS NOT NULL AND EXISTS (
                      SELECT 1 FROM community_members cm
                      WHERE cm.community_id = c.community_id AND cm.user_id = $2
                    ))
                    OR
                    -- DM channels: user must be a participant (cast enum to text for comparison)
                    (c.channel_type::text = 'direct_message' AND EXISTS (
                      SELECT 1 FROM direct_message_channels dm
                      WHERE dm.channel_id = c.id AND (dm.user1_id = $2 OR dm.user2_id = $2)
                    ))
                  )
                ORDER BY m.created_at DESC
                LIMIT $3
                "#,
                search_pattern,
                user_id,
                limit
            )
            .fetch_all(&self.db)
            .await?
        };

        Ok(messages)
    }

    /// Get reactions for multiple messages at once (more efficient)
    pub async fn get_reactions_for_messages(&self, message_ids: &[Uuid], user_id: Uuid) -> Result<std::collections::HashMap<Uuid, Vec<(String, Vec<Uuid>, bool)>>> {
        if message_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let reactions = sqlx::query!(
            r#"
            SELECT message_id, user_id, emoji
            FROM message_reactions
            WHERE message_id = ANY($1)
            ORDER BY message_id, emoji, created_at
            "#,
            message_ids
        )
        .fetch_all(&self.db)
        .await?;

        // Group by message_id, then by emoji
        let mut message_emoji_users: std::collections::HashMap<Uuid, std::collections::HashMap<String, Vec<Uuid>>> =
            std::collections::HashMap::new();

        for r in reactions {
            message_emoji_users
                .entry(r.message_id)
                .or_default()
                .entry(r.emoji)
                .or_default()
                .push(r.user_id);
        }

        // Convert to result format
        let mut result: std::collections::HashMap<Uuid, Vec<(String, Vec<Uuid>, bool)>> = std::collections::HashMap::new();
        for (message_id, emoji_users) in message_emoji_users {
            let mut reactions: Vec<(String, Vec<Uuid>, bool)> = emoji_users
                .into_iter()
                .map(|(emoji, user_ids)| {
                    let reacted_by_me = user_ids.contains(&user_id);
                    (emoji, user_ids, reacted_by_me)
                })
                .collect();
            reactions.sort_by(|a, b| a.0.cmp(&b.0));
            result.insert(message_id, reactions);
        }

        Ok(result)
    }

    /// Pin a message
    pub async fn pin_message(&self, message_id: Uuid, user_id: Uuid) -> Result<Message> {
        let message = sqlx::query_as!(
            Message,
            r#"
            UPDATE messages
            SET pinned_at = NOW(), pinned_by_id = $2
            WHERE id = $1
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            "#,
            message_id,
            user_id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

        Ok(message)
    }

    /// Unpin a message
    pub async fn unpin_message(&self, message_id: Uuid) -> Result<Message> {
        let message = sqlx::query_as!(
            Message,
            r#"
            UPDATE messages
            SET pinned_at = NULL, pinned_by_id = NULL
            WHERE id = $1
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            "#,
            message_id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

        Ok(message)
    }

    /// Get all pinned messages in a channel
    pub async fn get_pinned_messages(&self, channel_id: Uuid, limit: i64) -> Result<Vec<Message>> {
        let messages = sqlx::query_as!(
            Message,
            r#"
            SELECT id, channel_id, author_id, content, edited_at, reply_to_id, thread_parent_id, reply_count, last_reply_at, pinned_at, pinned_by_id, created_at
            FROM messages
            WHERE channel_id = $1 AND pinned_at IS NOT NULL
            ORDER BY pinned_at DESC
            LIMIT $2
            "#,
            channel_id,
            limit
        )
        .fetch_all(&self.db)
        .await?;

        Ok(messages)
    }
}
