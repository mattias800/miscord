use crate::auth::AuthUser;
use crate::error::{AppError, Result};
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use miscord_protocol::AttachmentData;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadResponse {
    pub attachments: Vec<AttachmentData>,
}

/// Upload files for a message
/// POST /api/channels/:channel_id/upload
/// Files are uploaded without being linked to a message initially.
/// They can be linked later using the link_to_message endpoint or by the client.
pub async fn upload_files(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(channel_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>> {
    // Verify channel exists
    let _channel = state.channel_service.get_by_id(channel_id).await?;

    // Ensure upload directory exists
    state.attachment_service.ensure_upload_dir().await?;

    let mut attachments = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("Failed to read multipart field: {}", e))
    })? {
        let filename = field
            .file_name()
            .map(String::from)
            .ok_or_else(|| AppError::BadRequest("Missing filename".to_string()))?;

        let content_type = field
            .content_type()
            .map(String::from)
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let data = field.bytes().await.map_err(|e| {
            AppError::BadRequest(format!("Failed to read file data: {}", e))
        })?;

        // Upload without linking to a message initially (message_id = None)
        let attachment = state
            .attachment_service
            .save_file(None, &filename, &content_type, &data)
            .await?;

        attachments.push(AttachmentData {
            id: attachment.id,
            filename: attachment.filename,
            content_type: attachment.content_type,
            size_bytes: attachment.size_bytes,
            url: attachment.url,
        });
    }

    if attachments.is_empty() {
        return Err(AppError::BadRequest("No files uploaded".to_string()));
    }

    Ok(Json(UploadResponse { attachments }))
}

/// Download/serve a file
/// GET /api/files/:id
pub async fn download_file(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response> {
    let attachment = state.attachment_service.get_by_id(id).await?;

    let data = state
        .attachment_service
        .read_file(id, &attachment.filename)
        .await?;

    // Determine if we should inline (display) or download
    let is_inline = attachment.content_type.starts_with("image/")
        || attachment.content_type.starts_with("video/")
        || attachment.content_type.starts_with("audio/")
        || attachment.content_type == "application/pdf";

    let disposition = if is_inline {
        format!("inline; filename=\"{}\"", attachment.filename)
    } else {
        format!("attachment; filename=\"{}\"", attachment.filename)
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &attachment.content_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CONTENT_LENGTH, data.len())
        .header(header::CACHE_CONTROL, "public, max-age=31536000") // Cache for 1 year
        .body(Body::from(data))
        .unwrap())
}

/// Get attachment metadata
/// GET /api/attachments/:id
pub async fn get_attachment(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<AttachmentData>> {
    let attachment = state.attachment_service.get_by_id(id).await?;

    Ok(Json(AttachmentData {
        id: attachment.id,
        filename: attachment.filename,
        content_type: attachment.content_type,
        size_bytes: attachment.size_bytes,
        url: attachment.url,
    }))
}

/// Delete an attachment
/// DELETE /api/attachments/:id
pub async fn delete_attachment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    let attachment = state.attachment_service.get_by_id(id).await?;

    // If attachment is linked to a message, check ownership
    if let Some(message_id) = attachment.message_id {
        let message = state.message_service.get_by_id(message_id).await?;

        // Only the message author can delete attachments
        if message.author_id != auth.user_id {
            return Err(AppError::Forbidden);
        }
    }
    // If not linked to a message, allow deletion (orphan attachment cleanup)

    state.attachment_service.delete(id).await?;

    Ok(StatusCode::NO_CONTENT)
}
