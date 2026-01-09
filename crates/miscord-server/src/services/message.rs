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
            INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, created_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, created_at
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
            SELECT id, channel_id, author_id, content, edited_at, reply_to_id, created_at
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
                SELECT id, channel_id, author_id, content, edited_at, reply_to_id, created_at
                FROM messages
                WHERE channel_id = $1 AND created_at < $2
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
                SELECT id, channel_id, author_id, content, edited_at, reply_to_id, created_at
                FROM messages
                WHERE channel_id = $1
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
            RETURNING id, channel_id, author_id, content, edited_at, reply_to_id, created_at
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

    pub async fn delete(&self, id: Uuid, author_id: Uuid) -> Result<()> {
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

        Ok(())
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

    /// Get reaction counts for a message
    pub async fn get_reactions(&self, message_id: Uuid, user_id: Uuid) -> Result<Vec<(String, i64, bool)>> {
        // Get all reactions grouped by emoji with count
        let reactions = sqlx::query!(
            r#"
            SELECT
                emoji,
                COUNT(*) as count,
                BOOL_OR(user_id = $2) as reacted_by_me
            FROM message_reactions
            WHERE message_id = $1
            GROUP BY emoji
            ORDER BY emoji
            "#,
            message_id,
            user_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(reactions
            .into_iter()
            .map(|r| (r.emoji, r.count.unwrap_or(0), r.reacted_by_me.unwrap_or(false)))
            .collect())
    }

    /// Get reactions for multiple messages at once (more efficient)
    pub async fn get_reactions_for_messages(&self, message_ids: &[Uuid], user_id: Uuid) -> Result<std::collections::HashMap<Uuid, Vec<(String, i64, bool)>>> {
        if message_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let reactions = sqlx::query!(
            r#"
            SELECT
                message_id,
                emoji,
                COUNT(*) as count,
                BOOL_OR(user_id = $2) as reacted_by_me
            FROM message_reactions
            WHERE message_id = ANY($1)
            GROUP BY message_id, emoji
            ORDER BY message_id, emoji
            "#,
            message_ids,
            user_id
        )
        .fetch_all(&self.db)
        .await?;

        let mut result: std::collections::HashMap<Uuid, Vec<(String, i64, bool)>> = std::collections::HashMap::new();
        for r in reactions {
            result
                .entry(r.message_id)
                .or_default()
                .push((r.emoji, r.count.unwrap_or(0), r.reacted_by_me.unwrap_or(false)));
        }

        Ok(result)
    }
}
