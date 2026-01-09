use chrono::{DateTime, Datelike, Local, Utc};
use eframe::egui;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;
use miscord_protocol::MessageData;

/// How often to send typing indicators (in seconds)
const TYPING_THROTTLE_SECS: u64 = 3;

/// Common reaction emojis
const REACTION_EMOJIS: &[&str] = &["👍", "❤️", "😂", "😮", "😢", "🎉"];

pub struct ChatView {
    message_input: String,
    /// Last time we sent a typing indicator
    last_typing_sent: Option<Instant>,
    /// Previous message input length (to detect changes)
    prev_input_len: usize,
    /// Message ID for which emoji picker is open
    emoji_picker_open_for: Option<Uuid>,
    /// Message being replied to
    replying_to: Option<MessageData>,
    /// Message being edited (stores original message)
    editing_message: Option<MessageData>,
}

/// Format a timestamp as relative time ("Just now", "2m ago", etc.)
fn format_relative_time(timestamp: DateTime<Utc>) -> String {
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
        // Show full date for older messages
        timestamp.with_timezone(&Local).format("%b %d, %Y").to_string()
    }
}

/// Format a full timestamp for tooltip
fn format_full_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.with_timezone(&Local).format("%B %d, %Y at %I:%M %p").to_string()
}

/// Get date separator text for a message
fn get_date_separator(timestamp: DateTime<Utc>) -> String {
    let now = Local::now();
    let local_time = timestamp.with_timezone(&Local);

    if local_time.date_naive() == now.date_naive() {
        "Today".to_string()
    } else if local_time.date_naive() == (now - chrono::Duration::days(1)).date_naive() {
        "Yesterday".to_string()
    } else if local_time.year() == now.year() {
        local_time.format("%B %d").to_string()
    } else {
        local_time.format("%B %d, %Y").to_string()
    }
}

/// Check if two messages are on different dates
fn is_different_date(msg1: &MessageData, msg2: &MessageData) -> bool {
    let local1 = msg1.created_at.with_timezone(&Local);
    let local2 = msg2.created_at.with_timezone(&Local);
    local1.date_naive() != local2.date_naive()
}

impl ChatView {
    pub fn new() -> Self {
        Self {
            message_input: String::new(),
            last_typing_sent: None,
            prev_input_len: 0,
            emoji_picker_open_for: None,
            replying_to: None,
            editing_message: None,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (current_channel, messages, channel_name, typing_usernames, message_reactions, current_user_id) = runtime.block_on(async {
            let s = state.read().await;
            let channel_id = s.current_channel_id;
            let messages = channel_id
                .and_then(|id| s.messages.get(&id))
                .cloned()
                .unwrap_or_default();
            let channel_name = channel_id
                .and_then(|id| s.channels.get(&id))
                .map(|c| c.name.clone())
                .unwrap_or_default();

            // Get current user ID for checking ownership and reactions
            let current_user_id = s.current_user.as_ref().map(|u| u.id);

            // Get all reactions for messages in this channel
            // Store as Vec for stable ordering, and include whether current user reacted
            let message_reactions: HashMap<Uuid, Vec<(String, usize, bool)>> = messages
                .iter()
                .filter_map(|msg| {
                    let reactions = s.message_reactions.get(&msg.id)?;
                    let mut counts: Vec<(String, usize, bool)> = reactions
                        .iter()
                        .map(|(emoji, users)| {
                            let i_reacted = current_user_id
                                .map(|uid| users.contains(&uid))
                                .unwrap_or(false);
                            (emoji.clone(), users.len(), i_reacted)
                        })
                        .collect();
                    // Sort by emoji for stable ordering
                    counts.sort_by(|a, b| a.0.cmp(&b.0));
                    if counts.is_empty() {
                        None
                    } else {
                        Some((msg.id, counts))
                    }
                })
                .collect();

            // Get typing users (excluding self)
            let current_user_id = s.current_user.as_ref().map(|u| u.id);
            let typing_users = if let Some(cid) = channel_id {
                state.get_typing_users(cid).await
            } else {
                vec![]
            };

            // Convert user IDs to usernames
            let typing_usernames: Vec<String> = typing_users
                .iter()
                .filter(|uid| current_user_id.map_or(true, |cuid| **uid != cuid))
                .filter_map(|uid| {
                    s.users.get(uid)
                        .map(|u| u.display_name.clone())
                        .or_else(|| {
                            s.members.values()
                                .flatten()
                                .find(|m| m.id == *uid)
                                .map(|m| m.display_name.clone())
                        })
                        .or_else(|| Some(format!("User {}", &uid.to_string()[..8])))
                })
                .collect();

            (channel_id, messages, channel_name, typing_usernames, message_reactions, current_user_id)
        });

        if current_channel.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select a channel to start chatting");
            });
            return;
        }

        let channel_id = current_channel.unwrap();

        // Use TopBottomPanel pattern within the central panel area
        // This ensures input stays at bottom and messages fill remaining space

        // Channel header at top
        egui::TopBottomPanel::top("chat_header")
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(format!("# {}", channel_name));
                });
            });

        // Input area at bottom (render first to reserve space)
        egui::TopBottomPanel::bottom("chat_input")
            .show_inside(ui, |ui| {
                // Typing indicator (always reserve space for consistent layout)
                ui.add_space(4.0);
                if !typing_usernames.is_empty() {
                    let typing_text = if typing_usernames.len() == 1 {
                        format!("{} is typing...", typing_usernames[0])
                    } else if typing_usernames.len() == 2 {
                        format!("{} and {} are typing...", typing_usernames[0], typing_usernames[1])
                    } else {
                        format!("{} and {} others are typing...", typing_usernames[0], typing_usernames.len() - 1)
                    };

                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(typing_text)
                                .small()
                                .italics()
                                .color(egui::Color32::from_rgb(140, 140, 140)),
                        );
                    });
                } else {
                    // Reserve space even when no one is typing
                    ui.label(
                        egui::RichText::new(" ")
                            .small(),
                    );
                }

                ui.separator();

                // Reply/Edit preview
                let mut cancel_reply = false;
                if let Some(reply_msg) = &self.replying_to {
                    let author_name = reply_msg.author_name.clone();
                    let content = reply_msg.content.clone();
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("↩ Replying to {}", author_name))
                                .small()
                                .color(egui::Color32::from_rgb(88, 101, 242)),
                        );
                        let preview: String = content.chars().take(50).collect();
                        ui.label(
                            egui::RichText::new(if content.len() > 50 {
                                format!("{}...", preview)
                            } else {
                                preview
                            })
                            .small()
                            .color(egui::Color32::from_rgb(140, 140, 140)),
                        );
                        if ui.small_button("✕").clicked() {
                            cancel_reply = true;
                        }
                    });
                }
                if cancel_reply {
                    self.replying_to = None;
                }

                let mut cancel_edit = false;
                if self.editing_message.is_some() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("✏ Editing message")
                                .small()
                                .color(egui::Color32::from_rgb(250, 166, 26)),
                        );
                        if ui.small_button("Cancel").clicked() {
                            cancel_edit = true;
                        }
                    });
                }
                if cancel_edit {
                    self.editing_message = None;
                    self.message_input.clear();
                }

                // Message input
                ui.horizontal(|ui| {
                    let hint_text = if self.editing_message.is_some() {
                        "Edit message (Shift+Enter for new line)".to_string()
                    } else {
                        format!("Message #{} (Shift+Enter for new line)", channel_name)
                    };

                    let response = ui.add(
                        egui::TextEdit::multiline(&mut self.message_input)
                            .hint_text(hint_text)
                            .desired_width(ui.available_width() - 60.0)
                            .desired_rows(2)
                            .lock_focus(true),
                    );

                    // Handle Enter (send) vs Shift+Enter (new line)
                    if response.has_focus() {
                        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let shift_held = ui.input(|i| i.modifiers.shift);

                        if enter_pressed && !shift_held {
                            // Remove the newline that was just inserted
                            if self.message_input.ends_with('\n') {
                                self.message_input.pop();
                            }
                            self.send_message(channel_id, state, network, runtime);
                        }
                    }

                    let btn_text = if self.editing_message.is_some() { "Save" } else { "Send" };
                    if ui.button(btn_text).clicked() {
                        self.send_message(channel_id, state, network, runtime);
                    }
                });
                ui.add_space(4.0);

                // Send typing indicator when user is typing
                let current_len = self.message_input.len();
                if current_len > self.prev_input_len && current_len > 0 {
                    // User is typing - send indicator if throttle period has passed
                    let should_send = self.last_typing_sent
                        .map(|t| t.elapsed().as_secs() >= TYPING_THROTTLE_SECS)
                        .unwrap_or(true);

                    if should_send {
                        let network = network.clone();
                        runtime.spawn(async move {
                            network.start_typing(channel_id).await;
                        });
                        self.last_typing_sent = Some(Instant::now());
                    }
                }
                self.prev_input_len = current_len;
            });

        // Messages area fills remaining space (CentralPanel fills what's left)
        egui::CentralPanel::default()
            .show_inside(ui, |ui| {
                // Build a lookup map for reply previews
                let message_lookup: HashMap<Uuid, &MessageData> = messages.iter().map(|m| (m.id, m)).collect();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let mut prev_message: Option<&MessageData> = None;

                        for message in messages.iter() {
                            // Show date separator if this is the first message or date changed
                            let show_separator = prev_message
                                .map(|prev| is_different_date(prev, message))
                                .unwrap_or(true);

                            if show_separator {
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    let separator_text = get_date_separator(message.created_at);
                                    ui.add(egui::Separator::default().horizontal());
                                    ui.label(
                                        egui::RichText::new(separator_text)
                                            .small()
                                            .color(egui::Color32::from_rgb(140, 140, 140)),
                                    );
                                    ui.add(egui::Separator::default().horizontal());
                                });
                                ui.add_space(8.0);
                            }

                            // Show reply preview if this is a reply
                            if let Some(reply_to_id) = message.reply_to_id {
                                if let Some(original_msg) = message_lookup.get(&reply_to_id) {
                                    // Truncate content for preview
                                    let preview_content: String = original_msg.content.chars().take(100).collect();
                                    let preview_content = if original_msg.content.len() > 100 {
                                        format!("{}...", preview_content)
                                    } else {
                                        preview_content
                                    };

                                    ui.horizontal(|ui| {
                                        ui.add_space(16.0);
                                        // Reply indicator line
                                        ui.label(
                                            egui::RichText::new("┌─")
                                                .color(egui::Color32::from_rgb(100, 100, 100)),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!("@{}", original_msg.author_name))
                                                .small()
                                                .strong()
                                                .color(egui::Color32::from_rgb(88, 101, 242)),
                                        );
                                        ui.label(
                                            egui::RichText::new(&preview_content)
                                                .small()
                                                .color(egui::Color32::from_rgb(160, 160, 160)),
                                        );
                                    });
                                } else {
                                    // Original message not found (possibly deleted or not loaded)
                                    ui.horizontal(|ui| {
                                        ui.add_space(16.0);
                                        ui.label(
                                            egui::RichText::new("┌─ Original message not available")
                                                .small()
                                                .italics()
                                                .color(egui::Color32::from_rgb(120, 120, 120)),
                                        );
                                    });
                                }
                            }

                            // Message header: author name, timestamp, and action buttons on right
                            let is_own_message = current_user_id.map_or(false, |uid| uid == message.author_id);

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
                                let reply_btn = ui.small_button("↩");
                                if reply_btn.clicked() {
                                    self.replying_to = Some(message.clone());
                                    self.editing_message = None;
                                }
                                reply_btn.on_hover_text("Reply");

                                // React button
                                let react_btn = ui.small_button("+");
                                let react_btn_clicked = react_btn.clicked();
                                let react_btn_rect = react_btn.rect;
                                react_btn.on_hover_text("Add reaction");

                                if react_btn_clicked {
                                    if self.emoji_picker_open_for == Some(message.id) {
                                        self.emoji_picker_open_for = None;
                                    } else {
                                        self.emoji_picker_open_for = Some(message.id);
                                    }
                                }

                                // Show emoji picker popup
                                if self.emoji_picker_open_for == Some(message.id) {
                                    let popup_id = egui::Id::new(format!("emoji_picker_{}", message.id));
                                    egui::Area::new(popup_id)
                                        .fixed_pos(egui::pos2(react_btn_rect.left(), react_btn_rect.bottom() + 2.0))
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
                                                                self.emoji_picker_open_for = None;
                                                            }
                                                        }
                                                    });
                                                });
                                        });
                                }

                                // Edit button (only for own messages)
                                if is_own_message {
                                    let edit_btn = ui.small_button("✏");
                                    if edit_btn.clicked() {
                                        self.editing_message = Some(message.clone());
                                        self.message_input = message.content.clone();
                                        self.replying_to = None;
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

                            // Message content with markdown rendering
                            ui.indent("msg_content", |ui| {
                                super::markdown::render_markdown(ui, &message.content);
                            });

                            // Display existing reactions (clickable to toggle)
                            if let Some(reactions) = message_reactions.get(&message.id) {
                                if !reactions.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(16.0);
                                        for (emoji, count, i_reacted) in reactions.iter() {
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
                                    });
                                }
                            }

                            ui.add_space(8.0);
                            prev_message = Some(message);
                        }
                    });
            });
    }

    fn send_message(
        &mut self,
        channel_id: uuid::Uuid,
        _state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        if self.message_input.trim().is_empty() {
            return;
        }

        let content = self.message_input.clone();
        self.message_input.clear();

        // Reset typing state
        self.last_typing_sent = None;
        self.prev_input_len = 0;

        let network = network.clone();

        // Check if we're editing or replying
        if let Some(edit_msg) = self.editing_message.take() {
            // Edit existing message
            runtime.spawn(async move {
                network.stop_typing(channel_id).await;
                if let Err(e) = network.update_message(edit_msg.id, &content).await {
                    tracing::warn!("Failed to update message: {}", e);
                }
            });
        } else {
            // Send new message (with optional reply)
            let reply_to_id = self.replying_to.take().map(|m| m.id);
            runtime.spawn(async move {
                network.stop_typing(channel_id).await;
                let _ = network.send_message_with_reply(channel_id, &content, reply_to_id).await;
            });
        }
    }
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}
