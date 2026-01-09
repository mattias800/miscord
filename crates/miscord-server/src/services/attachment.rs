use crate::error::{AppError, Result};
use crate::models::MessageAttachment;
use sqlx::PgPool;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Maximum file size: 25 MB
const MAX_FILE_SIZE: usize = 25 * 1024 * 1024;

/// Allowed file extensions
const ALLOWED_EXTENSIONS: &[&str] = &[
    // Images
    "jpg", "jpeg", "png", "gif", "webp", "svg", "ico", "bmp",
    // Documents
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
    "txt", "rtf", "csv", "md",
    // Archives
    "zip", "tar", "gz", "7z", "rar",
    // Audio
    "mp3", "wav", "ogg", "flac", "m4a", "aac",
    // Video
    "mp4", "webm", "mov", "avi", "mkv",
    // Code
    "js", "ts", "py", "rs", "go", "java", "c", "cpp", "h", "hpp", "cs",
    "html", "css", "json", "xml", "yaml", "yml", "toml", "sql", "sh",
];

#[derive(Clone)]
pub struct AttachmentService {
    db: PgPool,
    upload_dir: PathBuf,
    base_url: String,
}

impl AttachmentService {
    pub fn new(db: PgPool, upload_dir: PathBuf, base_url: String) -> Self {
        Self { db, upload_dir, base_url }
    }

    /// Ensure the upload directory exists
    pub async fn ensure_upload_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.upload_dir).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to create upload directory: {}", e))
        })?;
        Ok(())
    }

    /// Validate file before upload
    pub fn validate_file(&self, filename: &str, size: usize) -> Result<()> {
        // Check file size
        if size > MAX_FILE_SIZE {
            return Err(AppError::BadRequest(format!(
                "File too large. Maximum size is {} MB",
                MAX_FILE_SIZE / 1024 / 1024
            )));
        }

        // Check extension
        let extension = filename
            .rsplit('.')
            .next()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
            return Err(AppError::BadRequest(format!(
                "File type '{}' is not allowed",
                extension
            )));
        }

        Ok(())
    }

    /// Save a file and create database record
    /// message_id can be None for attachments uploaded before the message is created
    pub async fn save_file(
        &self,
        message_id: Option<Uuid>,
        filename: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<MessageAttachment> {
        self.validate_file(filename, data.len())?;

        let id = Uuid::new_v4();
        let extension = filename
            .rsplit('.')
            .next()
            .map(|s| format!(".{}", s.to_lowercase()))
            .unwrap_or_default();

        // Store with UUID-based filename to avoid conflicts
        let storage_filename = format!("{}{}", id, extension);
        let file_path = self.upload_dir.join(&storage_filename);

        // Write file to disk
        let mut file = fs::File::create(&file_path).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to create file: {}", e))
        })?;
        file.write_all(data).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to write file: {}", e))
        })?;

        // Create URL for the file
        let url = format!("/api/files/{}", id);

        // Insert into database
        // Note: Use "message_id as _" to tell sqlx the column is nullable
        let attachment = sqlx::query_as!(
            MessageAttachment,
            r#"
            INSERT INTO message_attachments (id, message_id, filename, content_type, size_bytes, url)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, message_id as "message_id: _", filename, content_type, size_bytes, url, created_at
            "#,
            id,
            message_id,
            filename,
            content_type,
            data.len() as i64,
            url
        )
        .fetch_one(&self.db)
        .await?;

        Ok(attachment)
    }

    /// Link attachments to a message (update message_id)
    pub async fn link_to_message(&self, attachment_ids: &[Uuid], message_id: Uuid) -> Result<()> {
        if attachment_ids.is_empty() {
            return Ok(());
        }

        sqlx::query!(
            r#"
            UPDATE message_attachments
            SET message_id = $1
            WHERE id = ANY($2)
            "#,
            message_id,
            attachment_ids
        )
        .execute(&self.db)
        .await?;

        Ok(())
    }

    /// Get attachment by ID
    pub async fn get_by_id(&self, id: Uuid) -> Result<MessageAttachment> {
        let attachment = sqlx::query_as!(
            MessageAttachment,
            r#"
            SELECT id, message_id as "message_id: _", filename, content_type, size_bytes, url, created_at
            FROM message_attachments WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&self.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Attachment not found".to_string()))?;

        Ok(attachment)
    }

    /// Get all attachments for a message
    pub async fn get_by_message_id(&self, message_id: Uuid) -> Result<Vec<MessageAttachment>> {
        let attachments = sqlx::query_as!(
            MessageAttachment,
            r#"
            SELECT id, message_id as "message_id: _", filename, content_type, size_bytes, url, created_at
            FROM message_attachments WHERE message_id = $1
            ORDER BY created_at ASC
            "#,
            message_id
        )
        .fetch_all(&self.db)
        .await?;

        Ok(attachments)
    }

    /// Get attachments for multiple messages (batch query)
    pub async fn get_by_message_ids(&self, message_ids: &[Uuid]) -> Result<Vec<MessageAttachment>> {
        if message_ids.is_empty() {
            return Ok(vec![]);
        }

        let attachments = sqlx::query_as!(
            MessageAttachment,
            r#"
            SELECT id, message_id as "message_id: _", filename, content_type, size_bytes, url, created_at
            FROM message_attachments WHERE message_id = ANY($1)
            ORDER BY created_at ASC
            "#,
            message_ids
        )
        .fetch_all(&self.db)
        .await?;

        Ok(attachments)
    }

    /// Get the file path on disk for an attachment
    pub fn get_file_path(&self, id: Uuid, filename: &str) -> PathBuf {
        let extension = filename
            .rsplit('.')
            .next()
            .map(|s| format!(".{}", s.to_lowercase()))
            .unwrap_or_default();

        self.upload_dir.join(format!("{}{}", id, extension))
    }

    /// Read file contents from disk
    pub async fn read_file(&self, id: Uuid, filename: &str) -> Result<Vec<u8>> {
        let path = self.get_file_path(id, filename);
        fs::read(&path).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to read file: {}", e))
        })
    }

    /// Delete an attachment (file and database record)
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let attachment = self.get_by_id(id).await?;

        // Delete file from disk
        let path = self.get_file_path(id, &attachment.filename);
        if path.exists() {
            fs::remove_file(&path).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to delete file: {}", e))
            })?;
        }

        // Delete from database
        sqlx::query!("DELETE FROM message_attachments WHERE id = $1", id)
            .execute(&self.db)
            .await?;

        Ok(())
    }
}
