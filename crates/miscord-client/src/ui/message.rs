//! Shared message rendering component used by both ChatView and ThreadPanel

use chrono::{DateTime, Local, Utc};
use eframe::egui;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;
use miscord_protocol::MessageData;

/// Common reaction emojis
pub const REACTION_EMOJIS: &[&str] = &["👍", "❤️", "😂", "😮", "😢", "🎉"];

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

/// Shared state for emoji picker across messages
pub struct MessageRendererState {
    /// Message ID for which emoji picker is open
    pub emoji_picker_open_for: Option<Uuid>,
}

impl MessageRendererState {
    pub fn new() -> Self {
        Self {
            emoji_picker_open_for: None,
        }
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
        // Author name
        ui.label(
            egui::RichText::new(&message.author_name)
                .strong()
                .color(egui::Color32::from_rgb(88, 101, 242)),
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

        // Action buttons - shown inline with small separator
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("|")
                .small()
                .color(egui::Color32::from_rgb(80, 80, 80)),
        );
        ui.add_space(4.0);

        // Reply button
        if options.show_reply_button {
            let reply_btn = ui.small_button("↩");
            if reply_btn.clicked() {
                action = Some(MessageAction::Reply(message.clone()));
            }
            reply_btn.on_hover_text("Reply");
        }

        // React button
        let react_btn = ui.small_button("+");
        if react_btn.clicked() {
            should_toggle_picker = true;
        }
        react_btn_rect = Some(react_btn.rect);
        react_btn.on_hover_text("Add reaction");

        // Thread button (only show if message doesn't have replies and option enabled)
        if options.show_thread_button && message.reply_count == 0 {
            let thread_btn = ui.small_button("🧵");
            if thread_btn.clicked() {
                action = Some(MessageAction::OpenThread(message.id));
            }
            thread_btn.on_hover_text("Start thread");
        }

        // Edit button (only for own messages)
        if is_own_message {
            let edit_btn = ui.small_button("✏");
            if edit_btn.clicked() {
                action = Some(MessageAction::Edit(message.clone()));
            }
            edit_btn.on_hover_text("Edit");
        }

        // Delete button (only for own messages)
        if is_own_message {
            let del_btn = ui.small_button("🗑");
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
                .fixed_pos(egui::pos2(btn_rect.left(), btn_rect.bottom() + 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for emoji in REACTION_EMOJIS {
                                    if ui.button(*emoji).clicked() {
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

    // Display existing reactions (clickable to toggle)
    // Use provided reactions parameter if available, otherwise fall back to message.reactions
    let has_reactions = reactions.map(|r| !r.is_empty()).unwrap_or(!message.reactions.is_empty());

    if has_reactions {
        ui.horizontal(|ui| {
            ui.add_space(16.0);

            if let Some(reaction_list) = reactions {
                // Use reactions from state (real-time updates)
                for (emoji, count, i_reacted) in reaction_list {
                    let reaction_text = format!("{} {}", emoji, count);
                    let fill_color = if *i_reacted {
                        egui::Color32::from_rgb(88, 101, 242)
                    } else {
                        egui::Color32::from_rgb(50, 50, 60)
                    };
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(reaction_text).small()
                        )
                        .small()
                        .fill(fill_color)
                        .rounding(egui::Rounding::same(12.0))
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
                    let reaction_text = format!("{} {}", reaction.emoji, reaction.count);
                    let fill_color = if reaction.reacted_by_me {
                        egui::Color32::from_rgb(88, 101, 242)
                    } else {
                        egui::Color32::from_rgb(50, 50, 60)
                    };
                    let btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(reaction_text).small()
                        )
                        .small()
                        .fill(fill_color)
                        .rounding(egui::Rounding::same(12.0))
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
                    egui::RichText::new(format!("🧵 {} · View thread", reply_text))
                        .small()
                        .color(egui::Color32::from_rgb(88, 101, 242))
                )
                .fill(egui::Color32::from_rgb(45, 45, 55))
                .rounding(egui::Rounding::same(4.0))
            );

            if thread_link.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            if thread_link.clicked() {
                action = Some(MessageAction::OpenThread(message.id));
            }

            if !time_text.is_empty() {
                ui.label(
                    egui::RichText::new(format!("Last reply {}", time_text))
                        .small()
                        .color(egui::Color32::from_rgb(140, 140, 140))
                );
            }
        });
    }

    action
}
