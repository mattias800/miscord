//! Shared message rendering component used by both ChatView and ThreadPanel

use chrono::{DateTime, Local, Utc};
use eframe::egui;
use std::sync::Arc;
use uuid::Uuid;

use crate::media::{AudioPlayer, AudioPlayerState, format_duration};
use crate::network::{NetworkClient, OpenGraphData};
use crate::state::AppState;
use miscord_protocol::MessageData;

/// Common reaction emojis - using simpler Unicode that renders well
pub const REACTION_EMOJIS: &[&str] = &["👍", "❤️", "😄", "😮", "😢", "🎉"];

/// Reaction button colors - deeper, richer palette
const REACTION_BG_INACTIVE: egui::Color32 = egui::Color32::from_rgb(45, 48, 54);
const REACTION_BG_ACTIVE: egui::Color32 = egui::Color32::from_rgb(62, 72, 186);  // Deep blue

/// Format a timestamp as relative time ("Just now", "2m ago", etc.)
pub fn format_relative_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration.num_seconds() < 60 {
        "Just now".to_string()
    } else if duration.num_minutes() < 60 {
        let mins = duration.num_minutes();
        format!("{}m ago", mins)
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{}h ago", hours)
    } else if duration.num_days() == 1 {
        "Yesterday".to_string()
    } else if duration.num_days() < 7 {
        let days = duration.num_days();
        format!("{}d ago", days)
    } else {
        timestamp.with_timezone(&Local).format("%b %d, %Y").to_string()
    }
}

/// Format a full timestamp for tooltip
pub fn format_full_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.with_timezone(&Local).format("%B %d, %Y at %I:%M %p").to_string()
}

/// Actions that can be triggered from a message
pub enum MessageAction {
    /// User wants to reply to this message
    Reply(MessageData),
    /// User wants to edit this message
    Edit(MessageData),
    /// User wants to open/start a thread on this message
    OpenThread(Uuid),
}

/// Options for rendering a message
pub struct MessageRenderOptions {
    /// Whether to show the thread button (for starting threads)
    pub show_thread_button: bool,
    /// Whether to show the thread indicator ("N replies - View thread")
    pub show_thread_indicator: bool,
    /// Whether to show the reply button
    pub show_reply_button: bool,
    /// Prefix for egui IDs to avoid conflicts
    pub id_prefix: &'static str,
}

impl Default for MessageRenderOptions {
    fn default() -> Self {
        Self {
            show_thread_button: true,
            show_thread_indicator: true,
            show_reply_button: true,
            id_prefix: "chat",
        }
    }
}

/// Download progress state
#[derive(Clone)]
pub enum DownloadProgress {
    /// Download in progress with percentage (0-100)
    Downloading(u8),
    /// Download completed
    Completed,
    /// Download failed
    Failed(String),
}

/// Lightbox state for viewing images full-size
#[derive(Clone)]
pub struct LightboxState {
    /// The URL/key of the image being viewed
    pub image_key: String,
    /// Original dimensions
    pub width: u32,
    pub height: u32,
}

/// Audio playback state for an attachment
pub struct AudioPlaybackState {
    pub attachment_id: Uuid,
    pub state: Arc<AudioPlayerState>,
    /// Cached audio data for this attachment
    pub data: Vec<u8>,
}

/// Shared state for emoji picker across messages
pub struct MessageRendererState {
    /// Message ID for which emoji picker is open
    pub emoji_picker_open_for: Option<Uuid>,
    /// Texture cache for link preview images (url -> texture handle)
    pub link_preview_textures: std::collections::HashMap<String, egui::TextureHandle>,
    /// Texture cache for attachment images (url -> texture handle)
    pub attachment_textures: std::collections::HashMap<String, egui::TextureHandle>,
    /// Download progress for attachments (attachment_id -> progress)
    pub download_progress: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<Uuid, DownloadProgress>>>,
    /// Audio player instance
    pub audio_player: Option<AudioPlayer>,
    /// Currently playing audio state
    pub audio_state: Option<AudioPlaybackState>,
    /// Lightbox state for viewing images full-size
    pub lightbox: Option<LightboxState>,
    /// Cached audio data for attachments (attachment_id -> data)
    pub audio_cache: std::collections::HashMap<Uuid, Vec<u8>>,
}

impl MessageRendererState {
    pub fn new() -> Self {
        Self {
            emoji_picker_open_for: None,
            link_preview_textures: std::collections::HashMap::new(),
            attachment_textures: std::collections::HashMap::new(),
            download_progress: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            audio_player: AudioPlayer::new().ok(),
            audio_state: None,
            lightbox: None,
            audio_cache: std::collections::HashMap::new(),
        }
    }
}

/// Format file size in human-readable format
pub fn format_file_size(bytes: i64) -> String {
    const KB: i64 = 1024;
    const MB: i64 = 1024 * 1024;
    const GB: i64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

impl Default for MessageRendererState {
    fn default() -> Self {
        Self::new()
    }
}

/// Reaction data for rendering (emoji, count, whether current user reacted)
pub type ReactionInfo = (String, usize, bool);

/// Render a single message with all its UI elements
/// Returns an optional MessageAction if the user triggered one
///
/// `reactions` parameter overrides `message.reactions` when provided (used for real-time updates)
pub fn render_message(
    ui: &mut egui::Ui,
    message: &MessageData,
    current_user_id: Option<Uuid>,
    reactions: Option<&[ReactionInfo]>,
    state: &AppState,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &mut MessageRendererState,
    options: &MessageRenderOptions,
) -> Option<MessageAction> {
    let mut action = None;
    let is_own_message = current_user_id.map_or(false, |uid| uid == message.author_id);

    // Track react button rect for emoji picker positioning
    let mut react_btn_rect: Option<egui::Rect> = None;
    let mut should_toggle_picker = false;

    // Message header: author name, timestamp, and action buttons
    ui.horizontal(|ui| {
        // Author name - using theme brand color
        ui.label(
            egui::RichText::new(&message.author_name)
                .strong()
                .color(egui::Color32::from_rgb(96, 165, 250)),  // Softer blue for names
        );

        // Timestamp
        let relative_time = format_relative_time(message.created_at);
        let full_time = format_full_timestamp(message.created_at);
        let time_label = ui.label(
            egui::RichText::new(&relative_time)
                .small()
                .color(egui::Color32::GRAY),
        );
        time_label.on_hover_text(&full_time);

        if message.edited_at.is_some() {
            ui.label(
                egui::RichText::new("(edited)")
                    .small()
                    .color(egui::Color32::from_rgb(160, 160, 160)),
            );
        }

        // Action buttons - subtle icons that appear on hover
        ui.add_space(12.0);
        ui.spacing_mut().item_spacing.x = 2.0;

        // Helper to create action buttons with consistent styling
        let action_btn = |ui: &mut egui::Ui, icon: &str, tooltip: &str| -> egui::Response {
            let btn = ui.add(
                egui::Button::new(
                    egui::RichText::new(icon)
                        .size(14.0)
                        .color(egui::Color32::from_rgb(180, 180, 180))
                )
                .fill(egui::Color32::TRANSPARENT)
                .min_size(egui::vec2(28.0, 24.0))
                .rounding(egui::Rounding::same(4.0))
            );
            btn.on_hover_text(tooltip)
        };

        // Reply button
        if options.show_reply_button {
            let reply_btn = action_btn(ui, "↩", "Reply");
            if reply_btn.clicked() {
                action = Some(MessageAction::Reply(message.clone()));
            }
        }

        // React button
        let react_btn = action_btn(ui, "😀", "Add reaction");
        if react_btn.clicked() {
            should_toggle_picker = true;
        }
        react_btn_rect = Some(react_btn.rect);

        // Thread button (only show if message doesn't have replies and option enabled)
        if options.show_thread_button && message.reply_count == 0 {
            let thread_btn = action_btn(ui, "💬", "Start thread");
            if thread_btn.clicked() {
                action = Some(MessageAction::OpenThread(message.id));
            }
        }

        // Edit button (only for own messages)
        if is_own_message {
            let edit_btn = action_btn(ui, "✎", "Edit");
            if edit_btn.clicked() {
                action = Some(MessageAction::Edit(message.clone()));
            }
        }

        // Delete button (only for own messages)
        if is_own_message {
            let del_btn = action_btn(ui, "✕", "Delete");
            if del_btn.clicked() {
                let network = network.clone();
                let msg_id = message.id;
                runtime.spawn(async move {
                    if let Err(e) = network.delete_message(msg_id).await {
                        tracing::warn!("Failed to delete message: {}", e);
                    }
                });
            }
            del_btn.on_hover_text("Delete");
        }
    });

    // Handle react button toggle (outside the horizontal block)
    if should_toggle_picker {
        if renderer_state.emoji_picker_open_for == Some(message.id) {
            renderer_state.emoji_picker_open_for = None;
        } else {
            renderer_state.emoji_picker_open_for = Some(message.id);
        }
    }

    // Show emoji picker popup (outside the horizontal block for proper layering)
    if renderer_state.emoji_picker_open_for == Some(message.id) {
        if let Some(btn_rect) = react_btn_rect {
            let popup_id = egui::Id::new(format!("{}_emoji_picker_{}", options.id_prefix, message.id));
            let mut close_picker = false;

            egui::Area::new(popup_id)
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(btn_rect.left(), btn_rect.bottom() + 4.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::same(8.0))
                        .rounding(egui::Rounding::same(8.0))
                        .fill(egui::Color32::from_rgb(30, 32, 36))  // BG_PRIMARY
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for emoji in REACTION_EMOJIS {
                                    let btn = ui.add(
                                        egui::Button::new(
                                            egui::RichText::new(*emoji).size(20.0)
                                        )
                                        .fill(egui::Color32::TRANSPARENT)
                                        .min_size(egui::vec2(36.0, 36.0))
                                        .rounding(egui::Rounding::same(6.0))
                                    );
                                    if btn.clicked() {
                                        let network = network.clone();
                                        let msg_id = message.id;
                                        let emoji_str = emoji.to_string();
                                        runtime.spawn(async move {
                                            if let Err(e) = network.add_reaction(msg_id, &emoji_str).await {
                                                tracing::warn!("Failed to add reaction: {}", e);
                                            }
                                        });
                                        close_picker = true;
                                    }
                                }
                            });
                        });
                });

            if close_picker {
                renderer_state.emoji_picker_open_for = None;
            }
        }
    }

    // Message content with markdown rendering
    ui.indent(format!("{}_msg_content_{}", options.id_prefix, message.id), |ui| {
        super::markdown::render_markdown(ui, &message.content);
    });

    // Link previews - extract URLs and show preview cards (limit to first URL only)
    let urls = super::markdown::extract_urls(&message.content);
    if !urls.is_empty() {
        // Get cached OpenGraph data for URLs (non-blocking)
        for url in &urls {
            // Check cache first (sync, non-blocking)
            let cached = state.get_opengraph_sync(url);

            if cached.is_none() {
                // Not cached - trigger fetch if not already pending (sync, non-blocking)
                if state.mark_opengraph_pending_sync(url) == Some(true) {
                    let network = network.clone();
                    let state = state.clone();
                    let url = url.clone();
                    runtime.spawn(async move {
                        match network.fetch_opengraph(&url).await {
                            Ok(data) => {
                                state.set_opengraph(url, data).await;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to fetch OpenGraph for {}: {}", url, e);
                                state.mark_opengraph_failed(&url).await;
                            }
                        }
                    });
                }
            }

            // Render preview card if we have cached data
            if let Some(og_data) = cached {
                // Only show preview if we have at least a title or description
                if og_data.title.is_some() || og_data.description.is_some() {
                    render_link_preview(ui, &og_data, state, network, runtime, renderer_state);
                }
            }
        }
    }

    // Display attachments
    if !message.attachments.is_empty() {
        ui.add_space(4.0);
        for attachment in &message.attachments {
            render_attachment(ui, attachment, state, network, runtime, renderer_state);
        }
    }

    // Display existing reactions (clickable to toggle)
    // Use provided reactions parameter if available, otherwise fall back to message.reactions
    let has_reactions = reactions.map(|r| !r.is_empty()).unwrap_or(!message.reactions.is_empty());

    if has_reactions {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.spacing_mut().item_spacing.x = 6.0;

            if let Some(reaction_list) = reactions {
                // Use reactions from state (real-time updates)
                for (emoji, count, i_reacted) in reaction_list {
                    let reaction_text = format!("{} {}", emoji, count);
                    let fill_color = if *i_reacted {
                        REACTION_BG_ACTIVE
                    } else {
                        REACTION_BG_INACTIVE
                    };
                    let text_color = if *i_reacted {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_rgb(220, 221, 222)
                    };
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(reaction_text)
                                .size(14.0)
                                .color(text_color)
                        )
                        .fill(fill_color)
                        .rounding(egui::Rounding::same(6.0))
                        .min_size(egui::vec2(0.0, 28.0))
                    );
                    if btn.clicked() {
                        let network = network.clone();
                        let msg_id = message.id;
                        let emoji_str = emoji.clone();
                        let should_remove = *i_reacted;
                        runtime.spawn(async move {
                            if should_remove {
                                if let Err(e) = network.remove_reaction(msg_id, &emoji_str).await {
                                    tracing::warn!("Failed to remove reaction: {}", e);
                                }
                            } else {
                                if let Err(e) = network.add_reaction(msg_id, &emoji_str).await {
                                    tracing::warn!("Failed to add reaction: {}", e);
                                }
                            }
                        });
                    }
                }
            } else {
                // Fall back to message.reactions (initial load)
                for reaction in &message.reactions {
                    let reaction_text = format!("{} {}", reaction.emoji, reaction.count());
                    let fill_color = if reaction.reacted_by_me {
                        REACTION_BG_ACTIVE
                    } else {
                        REACTION_BG_INACTIVE
                    };
                    let text_color = if reaction.reacted_by_me {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_rgb(220, 221, 222)
                    };
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(reaction_text)
                                .size(14.0)
                                .color(text_color)
                        )
                        .fill(fill_color)
                        .rounding(egui::Rounding::same(6.0))
                        .min_size(egui::vec2(0.0, 28.0))
                    );
                    if btn.clicked() {
                        let network = network.clone();
                        let msg_id = message.id;
                        let emoji_str = reaction.emoji.clone();
                        let should_remove = reaction.reacted_by_me;
                        runtime.spawn(async move {
                            if should_remove {
                                if let Err(e) = network.remove_reaction(msg_id, &emoji_str).await {
                                    tracing::warn!("Failed to remove reaction: {}", e);
                                }
                            } else {
                                if let Err(e) = network.add_reaction(msg_id, &emoji_str).await {
                                    tracing::warn!("Failed to add reaction: {}", e);
                                }
                            }
                        });
                    }
                }
            }
        });
    }

    // Thread indicator: "N replies - View thread"
    if options.show_thread_indicator && message.reply_count > 0 {
        ui.horizontal(|ui| {
            ui.add_space(16.0);

            let reply_text = if message.reply_count == 1 {
                "1 reply".to_string()
            } else {
                format!("{} replies", message.reply_count)
            };

            let time_text = message.last_reply_at
                .map(|t| format_relative_time(t))
                .unwrap_or_default();

            let thread_link = ui.add(
                egui::Button::new(
                    egui::RichText::new(format!("💬 {} · View thread", reply_text))
                        .size(13.0)
                        .color(egui::Color32::from_rgb(96, 165, 250))  // Softer blue
                )
                .fill(egui::Color32::from_rgb(38, 40, 46))  // BG_ELEVATED
                .rounding(egui::Rounding::same(6.0))
                .min_size(egui::vec2(0.0, 28.0))
            );

            if thread_link.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            if thread_link.clicked() {
                action = Some(MessageAction::OpenThread(message.id));
            }

            if !time_text.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("Last reply {}", time_text))
                        .size(12.0)
                        .color(egui::Color32::from_rgb(150, 150, 150))
                );
            }
        });
    }

    action
}

/// Render a link preview card with optional image
fn render_link_preview(
    ui: &mut egui::Ui,
    data: &OpenGraphData,
    state: &AppState,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &mut MessageRendererState,
) {
    ui.add_space(4.0);

    // Try to load image with actual network fetch
    let image_data = if let Some(image_url) = &data.image {
        let cached = state.get_image_sync(image_url);

        if cached.is_none() {
            if state.mark_image_pending_sync(image_url) == Some(true) {
                let network = network.clone();
                let state = state.clone();
                let url = image_url.clone();
                runtime.spawn(async move {
                    match network.fetch_image(&url).await {
                        Ok((bytes, width, height)) => {
                            state.set_image(url, bytes, width, height).await;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch image {}: {}", url, e);
                            state.mark_image_failed(&url).await;
                        }
                    }
                });
            }
        }

        cached
    } else {
        None
    };

    // Preview card with left accent bar (like Discord/Slack)
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(38, 40, 46))  // BG_ELEVATED
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::same(0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Left accent bar
                let content_height = if image_data.is_some() { 120.0 } else { 60.0 };
                let accent_rect = ui.allocate_exact_size(
                    egui::vec2(4.0, content_height),
                    egui::Sense::hover()
                ).0;
                ui.painter().rect_filled(
                    accent_rect,
                    egui::Rounding {
                        nw: 4.0,
                        sw: 4.0,
                        ne: 0.0,
                        se: 0.0,
                    },
                    egui::Color32::from_rgb(88, 101, 242),  // Discord blurple
                );

                // Content area
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.set_min_width(300.0);
                    ui.set_max_width(400.0);

                    // Site name (if available)
                    if let Some(site_name) = &data.site_name {
                        ui.label(
                            egui::RichText::new(site_name)
                                .size(11.0)
                                .color(egui::Color32::from_rgb(140, 140, 140))
                        );
                    }

                    // Author/channel name for videos
                    if let Some(author) = &data.author_name {
                        ui.label(
                            egui::RichText::new(author)
                                .size(12.0)
                                .color(egui::Color32::from_rgb(160, 160, 160))
                        );
                    }

                    // Title (clickable link)
                    if let Some(title) = &data.title {
                        let title_label = ui.add(
                            egui::Label::new(
                                egui::RichText::new(title)
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(0, 168, 252))  // Link blue
                            )
                            .sense(egui::Sense::click())
                        );

                        if title_label.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }

                        if title_label.clicked() {
                            if let Err(e) = open::that(&data.url) {
                                tracing::warn!("Failed to open URL: {}", e);
                            }
                        }

                        title_label.on_hover_text(&data.url);
                    }

                    // Description (truncated)
                    if let Some(description) = &data.description {
                        let truncated: String = description.chars().take(150).collect();
                        let display_text = if description.len() > 150 {
                            format!("{}...", truncated)
                        } else {
                            truncated
                        };

                        ui.label(
                            egui::RichText::new(display_text)
                                .size(13.0)
                                .color(egui::Color32::from_rgb(180, 180, 180))
                        );
                    }

                    // Image (if available and loaded)
                    if let Some(cached_img) = &image_data {
                        let image_url = data.image.as_ref().unwrap();
                        let (rgba_data, width, height) = cached_img.as_ref();
                        ui.add_space(8.0);

                        // Get or create texture from renderer state cache
                        if !renderer_state.link_preview_textures.contains_key(image_url) {
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [*width as usize, *height as usize],
                                rgba_data,
                            );
                            let handle = ui.ctx().load_texture(
                                format!("link_preview_{}", image_url),
                                color_image,
                                egui::TextureOptions::LINEAR,
                            );
                            renderer_state.link_preview_textures.insert(image_url.clone(), handle);
                        }

                        if let Some(texture) = renderer_state.link_preview_textures.get(image_url) {
                            // Calculate display size (max 300px wide, maintain aspect ratio)
                            let aspect = *width as f32 / *height as f32;
                            let display_width = (*width as f32).min(300.0);
                            let display_height = display_width / aspect;

                            let is_video = data.video_type.is_some();

                            // For videos, make the thumbnail clickable with a play button overlay
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(display_width, display_height),
                                egui::Sense::click(),
                            );

                            if ui.is_rect_visible(rect) {
                                // Draw the thumbnail
                                ui.painter().image(
                                    texture.id(),
                                    rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );

                                // Draw play button overlay for videos
                                if is_video {
                                    let center = rect.center();
                                    let play_radius = 24.0;

                                    // Semi-transparent dark circle
                                    ui.painter().circle_filled(
                                        center,
                                        play_radius,
                                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                                    );

                                    // Play triangle
                                    let triangle_size = 12.0;
                                    let triangle = vec![
                                        egui::pos2(center.x - triangle_size * 0.4, center.y - triangle_size),
                                        egui::pos2(center.x - triangle_size * 0.4, center.y + triangle_size),
                                        egui::pos2(center.x + triangle_size * 0.8, center.y),
                                    ];
                                    ui.painter().add(egui::Shape::convex_polygon(
                                        triangle,
                                        egui::Color32::WHITE,
                                        egui::Stroke::NONE,
                                    ));
                                }
                            }

                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            if response.clicked() {
                                if let Err(e) = open::that(&data.url) {
                                    tracing::warn!("Failed to open URL: {}", e);
                                }
                            }

                            response.on_hover_text(if is_video { "Click to watch video" } else { &data.url });
                        }
                    }

                    ui.add_space(8.0);
                });

                ui.add_space(12.0);
            });
        });
}

/// Render a file attachment (image inline, audio with player, other files as download cards)
fn render_attachment(
    ui: &mut egui::Ui,
    attachment: &miscord_protocol::AttachmentData,
    state: &AppState,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &mut MessageRendererState,
) {
    let is_image = attachment.content_type.starts_with("image/");
    let is_audio = attachment.content_type.starts_with("audio/");

    if is_image {
        render_image_attachment(ui, attachment, state, network, runtime, renderer_state);
    } else if is_audio {
        render_audio_attachment(ui, attachment, state, network, runtime, renderer_state);
    } else {
        render_file_attachment(ui, attachment, network, runtime, renderer_state);
    }
}

/// Render an image attachment inline
fn render_image_attachment(
    ui: &mut egui::Ui,
    attachment: &miscord_protocol::AttachmentData,
    state: &AppState,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &mut MessageRendererState,
) {
    // Use attachment URL as cache key
    let cache_key = &attachment.url;

    // Try to get cached image data
    let cached = state.get_image_sync(cache_key);

    if cached.is_none() {
        // Not cached - trigger fetch if not already pending
        if state.mark_image_pending_sync(cache_key) == Some(true) {
            let network = network.clone();
            let state = state.clone();
            let url = attachment.url.clone();
            runtime.spawn(async move {
                match network.fetch_attachment_image(&url).await {
                    Ok((bytes, width, height)) => {
                        state.set_image(url, bytes, width, height).await;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch attachment image {}: {}", url, e);
                        state.mark_image_failed(&url).await;
                    }
                }
            });
        }
    }

    // Render the image if we have cached data
    if let Some(cached_img) = cached {
        let (rgba_data, width, height) = cached_img.as_ref();

        // Get or create texture from renderer state cache
        if !renderer_state.attachment_textures.contains_key(cache_key) {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [*width as usize, *height as usize],
                rgba_data,
            );
            let handle = ui.ctx().load_texture(
                format!("attachment_{}", attachment.id),
                color_image,
                egui::TextureOptions::LINEAR,
            );
            renderer_state.attachment_textures.insert(cache_key.clone(), handle);
        }

        if let Some(texture) = renderer_state.attachment_textures.get(cache_key) {
            // Calculate display size (max 400px wide, maintain aspect ratio)
            let aspect = *width as f32 / *height as f32;
            let display_width = (*width as f32).min(400.0);
            let display_height = display_width / aspect;

            ui.add_space(4.0);

            // Make the image clickable to open in browser/viewer
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(display_width, display_height),
                egui::Sense::click(),
            );

            if ui.is_rect_visible(rect) {
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Show slight border on hover
                if response.hovered() {
                    ui.painter().rect_stroke(
                        rect,
                        egui::Rounding::same(4.0),
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(88, 101, 242)),
                    );
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }

            // Click to open lightbox
            if response.clicked() {
                renderer_state.lightbox = Some(LightboxState {
                    image_key: cache_key.clone(),
                    width: *width,
                    height: *height,
                });
            }

            // Show filename and size on hover
            response.on_hover_text(format!(
                "{} ({}) - Click to view full size",
                attachment.filename,
                format_file_size(attachment.size_bytes)
            ));
        }
    } else {
        // Show loading placeholder
        ui.add_space(4.0);
        egui::Frame::none()
            .fill(egui::Color32::from_rgb(38, 40, 46))
            .rounding(egui::Rounding::same(4.0))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!("Loading {}...", attachment.filename))
                            .color(egui::Color32::from_rgb(180, 180, 180))
                    );
                });
            });
    }
}

/// Render an audio attachment with inline player
fn render_audio_attachment(
    ui: &mut egui::Ui,
    attachment: &miscord_protocol::AttachmentData,
    state: &AppState,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &mut MessageRendererState,
) {
    ui.add_space(4.0);

    let attachment_id = attachment.id;

    // Check if we have cached audio data
    let has_data = renderer_state.audio_cache.contains_key(&attachment_id);

    // Check if this is the currently playing audio
    let is_current = renderer_state.audio_state
        .as_ref()
        .map(|s| s.attachment_id == attachment_id)
        .unwrap_or(false);

    // Get playback state for this attachment
    let playback_state = if is_current {
        renderer_state.audio_state.as_ref().map(|s| s.state.clone())
    } else {
        None
    };

    egui::Frame::none()
        .fill(egui::Color32::from_rgb(38, 40, 46))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(300.0);

            ui.horizontal(|ui| {
                // Play/Pause button (circular)
                let is_playing = playback_state.as_ref().map(|s| s.is_playing()).unwrap_or(false);
                let button_text = if is_playing { "⏸" } else { "▶" };

                let play_btn = ui.add_sized(
                    egui::vec2(36.0, 36.0),
                    egui::Button::new(
                        egui::RichText::new(button_text)
                            .size(18.0)
                            .color(egui::Color32::WHITE)
                    )
                    .fill(egui::Color32::from_rgb(88, 101, 242))
                    .rounding(egui::Rounding::same(18.0))
                );

                if play_btn.clicked() {
                    if is_current {
                        // Toggle play/pause
                        if let Some(player) = &mut renderer_state.audio_player {
                            player.toggle();
                        }
                    } else if has_data {
                        // Play from cache
                        if let Some(data) = renderer_state.audio_cache.get(&attachment_id).cloned() {
                            if let Some(player) = &mut renderer_state.audio_player {
                                match player.play(attachment_id, data.clone()) {
                                    Ok(state) => {
                                        renderer_state.audio_state = Some(AudioPlaybackState {
                                            attachment_id,
                                            state,
                                            data,
                                        });
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to play audio: {}", e);
                                    }
                                }
                            }
                        }
                    } else {
                        // Fetch and play
                        let network = network.clone();
                        let url = attachment.url.clone();
                        let state_clone = state.clone();

                        // Sync fetch for simplicity
                        let result = runtime.block_on(async {
                            network.fetch_attachment_raw(&url).await
                        });

                        match result {
                            Ok(data) => {
                                // Cache the data
                                renderer_state.audio_cache.insert(attachment_id, data.clone());

                                // Play it
                                if let Some(player) = &mut renderer_state.audio_player {
                                    match player.play(attachment_id, data.clone()) {
                                        Ok(state) => {
                                            renderer_state.audio_state = Some(AudioPlaybackState {
                                                attachment_id,
                                                state,
                                                data,
                                            });
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to play audio: {}", e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to fetch audio: {}", e);
                            }
                        }
                    }
                }

                ui.add_space(8.0);

                // Audio info and progress
                ui.vertical(|ui| {
                    // Filename
                    ui.label(
                        egui::RichText::new(&attachment.filename)
                            .size(14.0)
                            .color(egui::Color32::WHITE)
                    );

                    // Progress bar and time
                    if let Some(ps) = &playback_state {
                        let mut progress = ps.get_position_ms() as f32 / ps.duration_ms.max(1) as f32;

                        // Time display
                        ui.label(
                            egui::RichText::new(format!(
                                "{} / {}",
                                format_duration(ps.get_position_ms()),
                                format_duration(ps.duration_ms)
                            ))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(180, 180, 180))
                        );

                        // Progress slider (draggable, seek on release)
                        let seek_slider = egui::Slider::new(&mut progress, 0.0..=1.0)
                            .show_value(false)
                            .trailing_fill(true);

                        let slider_response = ui.add_sized(egui::vec2(ui.available_width().min(200.0), 18.0), seek_slider);
                        if slider_response.drag_stopped() {
                            if let Some(player) = &mut renderer_state.audio_player {
                                let _ = player.seek(progress);
                            }
                        }

                        // Volume control
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("🔊")
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(140, 140, 140))
                            );

                            let mut volume = ps.get_volume();
                            let slider = egui::Slider::new(&mut volume, 0.0..=1.0)
                                .show_value(false)
                                .trailing_fill(true);

                            if ui.add_sized(egui::vec2(60.0, 14.0), slider).changed() {
                                if let Some(player) = &mut renderer_state.audio_player {
                                    player.set_volume(volume);
                                }
                            }
                        });
                    } else {
                        // Show file size when not playing
                        ui.label(
                            egui::RichText::new(format_file_size(attachment.size_bytes))
                                .size(12.0)
                                .color(egui::Color32::from_rgb(140, 140, 140))
                        );
                    }
                });
            });
        });

    // Request repaint while playing for animation
    if playback_state.as_ref().map(|s| s.is_playing()).unwrap_or(false) {
        ui.ctx().request_repaint();
    }
}

/// Render a non-image file attachment as a download card
fn render_file_attachment(
    ui: &mut egui::Ui,
    attachment: &miscord_protocol::AttachmentData,
    network: &NetworkClient,
    runtime: &tokio::runtime::Runtime,
    renderer_state: &MessageRendererState,
) {
    ui.add_space(4.0);

    // Get file extension for icon selection
    let extension = attachment.filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();

    // Choose icon based on file type
    let icon = match extension.as_str() {
        "pdf" => "📄",
        "doc" | "docx" => "📝",
        "xls" | "xlsx" => "📊",
        "ppt" | "pptx" => "📽",
        "zip" | "rar" | "7z" | "tar" | "gz" => "📦",
        "mp3" | "wav" | "ogg" | "flac" => "🎵",
        "mp4" | "mov" | "avi" | "mkv" | "webm" => "🎬",
        "txt" | "md" | "json" | "xml" | "yaml" | "yml" => "📃",
        "rs" | "py" | "js" | "ts" | "java" | "c" | "cpp" | "h" | "go" => "💻",
        _ => "📎",
    };

    // Check current download progress for this attachment
    let current_progress = renderer_state
        .download_progress
        .read()
        .ok()
        .and_then(|map| map.get(&attachment.id).cloned());

    let is_downloading = matches!(current_progress, Some(DownloadProgress::Downloading(_)));

    egui::Frame::none()
        .fill(egui::Color32::from_rgb(38, 40, 46))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(200.0);
            ui.set_max_width(300.0);

            ui.horizontal(|ui| {
                // File icon
                ui.label(egui::RichText::new(icon).size(24.0));

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    // Filename (clickable, but disabled during download)
                    let filename_label = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&attachment.filename)
                                .size(14.0)
                                .color(if is_downloading {
                                    egui::Color32::from_rgb(140, 140, 140)
                                } else {
                                    egui::Color32::from_rgb(96, 165, 250)
                                })
                        )
                        .sense(if is_downloading {
                            egui::Sense::hover()
                        } else {
                            egui::Sense::click()
                        })
                    );

                    if !is_downloading && filename_label.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    if !is_downloading && filename_label.clicked() {
                        // Trigger download with progress tracking
                        let network = network.clone();
                        let url = attachment.url.clone();
                        let filename = attachment.filename.clone();
                        let attachment_id = attachment.id;
                        let total_size = attachment.size_bytes;
                        let progress_map = renderer_state.download_progress.clone();

                        // Set initial progress
                        if let Ok(mut map) = progress_map.write() {
                            map.insert(attachment_id, DownloadProgress::Downloading(0));
                        }

                        runtime.spawn(async move {
                            match download_attachment_with_progress(
                                &network,
                                &url,
                                &filename,
                                total_size,
                                attachment_id,
                                progress_map.clone(),
                            ).await {
                                Ok(()) => {
                                    if let Ok(mut map) = progress_map.write() {
                                        map.insert(attachment_id, DownloadProgress::Completed);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to download attachment: {}", e);
                                    if let Ok(mut map) = progress_map.write() {
                                        map.insert(attachment_id, DownloadProgress::Failed(e.to_string()));
                                    }
                                }
                            }
                        });
                    }

                    // File size and progress info
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format_file_size(attachment.size_bytes))
                                .size(12.0)
                                .color(egui::Color32::from_rgb(140, 140, 140))
                        );

                        // Show download progress if applicable
                        match &current_progress {
                            Some(DownloadProgress::Downloading(percent)) => {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(format!("Downloading... {}%", percent))
                                        .size(12.0)
                                        .color(egui::Color32::from_rgb(96, 165, 250))
                                );
                            }
                            Some(DownloadProgress::Completed) => {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("Downloaded")
                                        .size(12.0)
                                        .color(egui::Color32::from_rgb(80, 200, 120))
                                );
                            }
                            Some(DownloadProgress::Failed(_)) => {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("Failed")
                                        .size(12.0)
                                        .color(egui::Color32::from_rgb(240, 80, 80))
                                );
                            }
                            None => {}
                        }
                    });
                });
            });
        });

    // Request repaint while downloading to update progress
    if is_downloading {
        ui.ctx().request_repaint();
    }
}

/// Download an attachment with progress tracking and save to the user's downloads folder
async fn download_attachment_with_progress(
    network: &NetworkClient,
    attachment_url: &str,
    filename: &str,
    total_size: i64,
    attachment_id: Uuid,
    progress_map: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<Uuid, DownloadProgress>>>,
) -> anyhow::Result<()> {
    use anyhow::Context;
    use futures_util::StreamExt;

    let server_url = network.get_base_url().await;
    let full_url = format!("{}{}", server_url, attachment_url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(&full_url).send().await?;

    // Get content length from response if available, otherwise use total_size
    let content_length = response
        .content_length()
        .unwrap_or(total_size as u64);

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut all_bytes = Vec::with_capacity(content_length as usize);

    // Stream the download and update progress
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        downloaded += chunk.len() as u64;
        all_bytes.extend_from_slice(&chunk);

        // Calculate percentage
        let percent = if content_length > 0 {
            ((downloaded as f64 / content_length as f64) * 100.0).min(100.0) as u8
        } else {
            0
        };

        // Update progress map
        if let Ok(mut map) = progress_map.write() {
            map.insert(attachment_id, DownloadProgress::Downloading(percent));
        }
    }

    // Get downloads directory
    let downloads_dir = dirs::download_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let file_path = downloads_dir.join(filename);

    // Write file
    std::fs::write(&file_path, &all_bytes)
        .with_context(|| format!("Failed to write file to {:?}", file_path))?;

    tracing::info!("Downloaded attachment to {:?}", file_path);

    // Try to open the containing folder
    if let Err(e) = open::that(&downloads_dir) {
        tracing::warn!("Failed to open downloads folder: {}", e);
    }

    Ok(())
}

/// Render the image lightbox overlay if an image is being viewed
/// This should be called after rendering all other UI elements
pub fn render_lightbox(
    ctx: &egui::Context,
    renderer_state: &mut MessageRendererState,
) {
    // Check if lightbox should be shown
    let lightbox_data = match &renderer_state.lightbox {
        Some(l) => l.clone(),
        None => return,
    };

    let LightboxState { image_key, width, height } = lightbox_data;

    // Get the texture
    let texture = match renderer_state.attachment_textures.get(&image_key) {
        Some(t) => t.clone(),
        None => {
            renderer_state.lightbox = None;
            return;
        }
    };

    // Handle escape key first
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        renderer_state.lightbox = None;
        return;
    }

    // Calculate image display dimensions
    let screen = ctx.screen_rect();
    let max_width = screen.width() * 0.9;
    let max_height = screen.height() * 0.9;

    let aspect = width as f32 / height as f32;
    let mut display_width = width as f32;
    let mut display_height = height as f32;

    if display_width > max_width {
        display_width = max_width;
        display_height = display_width / aspect;
    }
    if display_height > max_height {
        display_height = max_height;
        display_width = display_height * aspect;
    }

    let image_rect = egui::Rect::from_center_size(
        screen.center(),
        egui::vec2(display_width, display_height),
    );

    // Use a single modal-style window approach instead of multiple areas
    let modal_response = egui::Area::new(egui::Id::new("lightbox_modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(0.0, 0.0))
        .show(ctx, |ui| {
            let screen_rect = ui.ctx().screen_rect();

            // Allocate the full screen for backdrop interaction
            let backdrop_response = ui.allocate_rect(screen_rect, egui::Sense::click());

            // Dark backdrop
            ui.painter().rect_filled(
                screen_rect,
                egui::Rounding::ZERO,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 220),
            );

            // Draw the image (not interactive, we handle clicks via backdrop)
            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );

            // Subtle border around image
            ui.painter().rect_stroke(
                image_rect,
                egui::Rounding::same(4.0),
                egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 70)),
            );

            // Close button
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(screen_rect.right() - 55.0, 15.0),
                egui::vec2(40.0, 40.0),
            );

            let close_response = ui.allocate_rect(close_rect, egui::Sense::click());

            // Draw close button
            ui.painter().rect_filled(
                close_rect,
                egui::Rounding::same(20.0),
                if close_response.hovered() {
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40)
                } else {
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100)
                },
            );

            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "✕",
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );

            // Return whether to close
            backdrop_response.clicked() || close_response.clicked()
        });

    // Close if clicked anywhere (backdrop or close button)
    if modal_response.inner {
        renderer_state.lightbox = None;
    }
}
