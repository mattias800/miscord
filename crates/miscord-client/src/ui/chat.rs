use chrono::{DateTime, Datelike, Local, Utc};
use eframe::egui;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;
use miscord_protocol::MessageData;

use super::gif_picker::GifPicker;
use super::message::{
    format_file_size, format_relative_time, render_lightbox, render_message, MessageAction,
    MessageRenderOptions, MessageRendererState, ReactionInfo,
};

/// How often to send typing indicators (in seconds)
const TYPING_THROTTLE_SECS: u64 = 3;

/// Pending file attachment (filename, content_type, data)
pub struct PendingAttachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

pub struct ChatView {
    message_input: String,
    /// Last time we sent a typing indicator
    last_typing_sent: Option<Instant>,
    /// Previous message input length (to detect changes)
    prev_input_len: usize,
    /// Message being replied to
    replying_to: Option<MessageData>,
    /// Message being edited (stores original message)
    editing_message: Option<MessageData>,
    /// Shared message renderer state
    renderer_state: MessageRendererState,
    /// Whether mention autocomplete is active
    mention_active: bool,
    /// Current mention search query
    mention_query: String,
    /// Selected mention index in dropdown
    mention_selected: usize,
    /// Cursor position to set after mention insertion (char index)
    pending_cursor_pos: Option<usize>,
    /// Whether mention was dismissed with Escape (prevents immediate re-open)
    mention_dismissed: bool,
    /// Pending file attachments to upload with the next message
    pending_attachments: Vec<PendingAttachment>,
    /// Currently viewed channel (for draft save/restore on channel switch)
    current_channel_id: Option<Uuid>,
    /// Whether we're currently loading older messages
    loading_history: bool,
    /// Whether we've reached the end of message history (no more to load)
    reached_history_end: bool,
    /// Track if user has scrolled away from bottom (for "Jump to present" button)
    scrolled_to_bottom: bool,
    /// Request to jump to bottom on next frame
    jump_to_bottom_requested: bool,
    /// Track message count to detect when loading completes
    last_message_count: usize,
    /// Time when we started loading history (for timeout)
    loading_started_at: Option<Instant>,
    /// Currently selected message for keyboard navigation
    selected_message_id: Option<Uuid>,
    /// Whether the pinned messages panel is open
    show_pinned_panel: bool,
    /// Cached pinned messages for the current channel
    pinned_messages: Vec<MessageData>,
    /// Whether we're loading pinned messages
    pinned_messages_loading: bool,
    /// GIF picker state
    gif_picker: GifPicker,
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
            replying_to: None,
            editing_message: None,
            renderer_state: MessageRendererState::new(),
            mention_active: false,
            mention_query: String::new(),
            mention_selected: 0,
            pending_cursor_pos: None,
            mention_dismissed: false,
            pending_attachments: Vec::new(),
            current_channel_id: None,
            loading_history: false,
            reached_history_end: false,
            scrolled_to_bottom: true,
            jump_to_bottom_requested: false,
            last_message_count: 0,
            loading_started_at: None,
            selected_message_id: None,
            show_pinned_panel: false,
            pinned_messages: Vec::new(),
            pinned_messages_loading: false,
            gif_picker: GifPicker::new(),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        // Handle dropped files (drag-and-drop)
        self.handle_dropped_files(ui);

        // Handle draft save/restore on channel switch
        let new_channel_id = runtime.block_on(async {
            state.read().await.current_channel_id
        });

        if new_channel_id != self.current_channel_id {
            // Channel changed - save draft for old channel, load draft for new
            if let Some(old_channel_id) = self.current_channel_id {
                // Save current input as draft for the old channel
                let draft = self.message_input.clone();
                let state_clone = state.clone();
                runtime.spawn(async move {
                    state_clone.save_draft(old_channel_id, draft).await;
                });
            }

            if let Some(channel_id) = new_channel_id {
                // Load draft for new channel
                if let Some(draft) = runtime.block_on(state.get_draft(channel_id)) {
                    self.message_input = draft;
                } else {
                    self.message_input.clear();
                }
            } else {
                self.message_input.clear();
            }

            // Clear reply/edit state when switching channels
            self.replying_to = None;
            self.editing_message = None;
            self.mention_active = false;
            self.current_channel_id = new_channel_id;
            // Reset history state for new channel
            self.loading_history = false;
            self.reached_history_end = false;
            self.scrolled_to_bottom = true;
            self.jump_to_bottom_requested = false;
            self.last_message_count = 0;
            self.loading_started_at = None;
            self.selected_message_id = None;
            // Reset pinned messages panel for new channel
            self.show_pinned_panel = false;
            self.pinned_messages.clear();
            self.pinned_messages_loading = false;
        }

        let (current_channel, messages, channel_name, typing_usernames, current_user_id, message_reactions, members, scroll_to_message_id) = runtime.block_on(async {
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

            // Get members for mention autocomplete
            let members: Vec<(Uuid, String, String)> = s.current_community_id
                .and_then(|cid| s.members.get(&cid))
                .map(|m| m.iter().map(|u| (u.id, u.username.clone(), u.display_name.clone())).collect())
                .unwrap_or_default();

            // Get current user ID for checking ownership and reactions
            let current_user_id = s.current_user.as_ref().map(|u| u.id);

            // Get all reactions for messages in this channel
            let message_reactions: HashMap<Uuid, Vec<ReactionInfo>> = messages
                .iter()
                .filter_map(|msg| {
                    let reactions = s.message_reactions.get(&msg.id)?;
                    let mut counts: Vec<ReactionInfo> = reactions
                        .iter()
                        .map(|(emoji, reaction_state)| {
                            let i_reacted = current_user_id
                                .map(|uid| reaction_state.has_user(uid))
                                .unwrap_or(false);
                            (emoji.clone(), reaction_state.count(), i_reacted)
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

            // Get scroll target
            let scroll_to_message_id = s.scroll_to_message_id;

            (channel_id, messages, channel_name, typing_usernames, current_user_id, message_reactions, members, scroll_to_message_id)
        });

        // Handle keyboard navigation for messages
        self.handle_keyboard_navigation(ui, &messages, current_user_id, state, network, runtime);

        if current_channel.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select a channel to start chatting");
            });
            return;
        }

        let channel_id = current_channel.unwrap();

        // Load pinned messages if panel is open and we haven't loaded yet
        if self.show_pinned_panel && self.pinned_messages.is_empty() && !self.pinned_messages_loading {
            self.pinned_messages_loading = true;
            let ch_id = channel_id;
            match runtime.block_on(network.get_pinned_messages(ch_id)) {
                Ok(messages) => {
                    self.pinned_messages = messages;
                    self.pinned_messages_loading = false;
                }
                Err(e) => {
                    tracing::warn!("Failed to load pinned messages: {}", e);
                    self.pinned_messages_loading = false;
                }
            }
        }

        // Channel header at top
        egui::TopBottomPanel::top("chat_header")
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(format!("# {}", channel_name));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Pinned messages button
                        let pinned_count = self.pinned_messages.len();
                        let pin_btn_text = if self.show_pinned_panel {
                            "📌 Hide Pinned".to_string()
                        } else if pinned_count > 0 {
                            format!("📌 Pinned ({})", pinned_count)
                        } else {
                            "📌 Pinned".to_string()
                        };
                        let pin_btn = ui.add(
                            egui::Button::new(
                                egui::RichText::new(&pin_btn_text)
                                    .size(13.0)
                            )
                            .rounding(egui::Rounding::same(4.0))
                        );
                        if pin_btn.clicked() {
                            self.show_pinned_panel = !self.show_pinned_panel;
                        }
                    });
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

                // Formatting toolbar
                ui.horizontal(|ui| {
                    ui.add_space(4.0);

                    // Bold button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("B").strong().size(13.0))
                            .min_size(egui::vec2(28.0, 24.0))
                    ).on_hover_text("Bold (Ctrl+B)").clicked() {
                        self.insert_formatting("**", "**");
                    }

                    // Italic button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("I").italics().size(13.0))
                            .min_size(egui::vec2(28.0, 24.0))
                    ).on_hover_text("Italic (Ctrl+I)").clicked() {
                        self.insert_formatting("*", "*");
                    }

                    // Strikethrough button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("S").strikethrough().size(13.0))
                            .min_size(egui::vec2(28.0, 24.0))
                    ).on_hover_text("Strikethrough").clicked() {
                        self.insert_formatting("~~", "~~");
                    }

                    ui.separator();

                    // Inline code button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("</>").monospace().size(12.0))
                            .min_size(egui::vec2(32.0, 24.0))
                    ).on_hover_text("Inline code").clicked() {
                        self.insert_formatting("`", "`");
                    }

                    // Code block button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("```").monospace().size(11.0))
                            .min_size(egui::vec2(36.0, 24.0))
                    ).on_hover_text("Code block").clicked() {
                        self.insert_formatting("```\n", "\n```");
                    }

                    ui.separator();

                    // Link button
                    if ui.add(
                        egui::Button::new(egui::RichText::new("🔗").size(13.0))
                            .min_size(egui::vec2(28.0, 24.0))
                    ).on_hover_text("Insert link [text](url)").clicked() {
                        self.insert_formatting("[", "](url)");
                    }

                    ui.separator();

                    // GIF button
                    let gif_button = ui.add(
                        egui::Button::new(egui::RichText::new("GIF").size(12.0))
                            .min_size(egui::vec2(36.0, 24.0))
                    ).on_hover_text("Search GIFs");
                    if gif_button.clicked() {
                        self.gif_picker.toggle();
                    }
                });

                ui.add_space(4.0);

                // Update mention state before handling keys
                self.update_mention_state();

                // Build matching members list for mention autocomplete
                let matching_members: Vec<_> = if self.mention_active {
                    let query_lower = self.mention_query.to_lowercase();
                    members
                        .iter()
                        .filter(|(_, username, display_name)| {
                            query_lower.is_empty()
                                || username.to_lowercase().contains(&query_lower)
                                || display_name.to_lowercase().contains(&query_lower)
                        })
                        .take(5)
                        .cloned()
                        .collect()
                } else {
                    vec![]
                };

                // Handle mention keyboard navigation BEFORE text input
                // This way we can intercept the keys
                let mut mention_handled = false;
                let mut refocus_input = false;
                if self.mention_active && !matching_members.is_empty() {
                    let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                    let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                    let tab = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
                    let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

                    if up && self.mention_selected > 0 {
                        self.mention_selected -= 1;
                        mention_handled = true;
                    }
                    if down && self.mention_selected < matching_members.len().saturating_sub(1) {
                        self.mention_selected += 1;
                        mention_handled = true;
                    }
                    if (tab || enter) && !matching_members.is_empty() {
                        let (_, username, _) = &matching_members[self.mention_selected];
                        self.insert_mention(username);
                        mention_handled = true;
                    }
                    if escape {
                        self.mention_active = false;
                        self.mention_dismissed = true;  // Prevent immediate re-open
                        self.mention_selected = 0;
                        mention_handled = true;
                        refocus_input = true;  // Re-focus the text input after closing dropdown
                    }
                }

                // Show pending attachments above input
                if !self.pending_attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        let mut to_remove = Vec::new();
                        for (idx, attachment) in self.pending_attachments.iter().enumerate() {
                            egui::Frame::none()
                                .fill(egui::Color32::from_rgb(45, 48, 54))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::same(8.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        // File icon
                                        let icon = if attachment.content_type.starts_with("image/") {
                                            "🖼"
                                        } else {
                                            "📎"
                                        };
                                        ui.label(egui::RichText::new(icon).size(16.0));

                                        // Filename (truncated)
                                        let display_name: String = if attachment.filename.len() > 20 {
                                            format!("{}...", &attachment.filename[..17])
                                        } else {
                                            attachment.filename.clone()
                                        };
                                        ui.label(
                                            egui::RichText::new(&display_name)
                                                .size(12.0)
                                                .color(egui::Color32::from_rgb(200, 200, 200))
                                        );

                                        // Size
                                        ui.label(
                                            egui::RichText::new(format_file_size(attachment.data.len() as i64))
                                                .size(11.0)
                                                .color(egui::Color32::from_rgb(140, 140, 140))
                                        );

                                        // Remove button
                                        if ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("✕")
                                                    .size(12.0)
                                                    .color(egui::Color32::from_rgb(180, 180, 180))
                                            )
                                            .fill(egui::Color32::TRANSPARENT)
                                            .min_size(egui::vec2(20.0, 20.0))
                                        ).on_hover_text("Remove attachment").clicked() {
                                            to_remove.push(idx);
                                        }
                                    });
                                });
                            ui.add_space(4.0);
                        }
                        // Remove attachments marked for removal
                        for idx in to_remove.into_iter().rev() {
                            self.pending_attachments.remove(idx);
                        }
                    });
                    ui.add_space(4.0);
                }

                // Message input
                let text_edit_id = ui.make_persistent_id("chat_message_input");
                let input_row_response = ui.horizontal(|ui| {
                    // Attachment button (only when not editing)
                    if self.editing_message.is_none() {
                        let attach_btn = ui.add(
                            egui::Button::new(
                                egui::RichText::new("📎")
                                    .size(18.0)
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .min_size(egui::vec2(32.0, 32.0))
                            .rounding(egui::Rounding::same(4.0))
                        );

                        if attach_btn.on_hover_text("Attach file").clicked() {
                            self.open_file_picker();
                        }
                    }

                    let hint_text = if self.editing_message.is_some() {
                        "Edit message (Shift+Enter for new line)".to_string()
                    } else {
                        format!("Message #{} (Shift+Enter for new line)", channel_name)
                    };

                    let response = ui.add(
                        egui::TextEdit::multiline(&mut self.message_input)
                            .id(text_edit_id)
                            .hint_text(hint_text)
                            .desired_width(ui.available_width() - 60.0)
                            .desired_rows(2)
                            .lock_focus(true),
                    );

                    // Apply pending cursor position after mention insertion
                    if let Some(cursor_pos) = self.pending_cursor_pos.take() {
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), text_edit_id) {
                            let ccursor = egui::text::CCursor::new(cursor_pos);
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                            state.store(ui.ctx(), text_edit_id);
                        }
                    }

                    // Handle Enter (send) vs Shift+Enter (new line)
                    // Only if mention autocomplete didn't handle it
                    if response.has_focus() && !mention_handled {
                        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let shift_held = ui.input(|i| i.modifiers.shift);

                        if enter_pressed && !shift_held {
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

                // Re-focus text input after closing mention dropdown with Escape
                if refocus_input {
                    ui.memory_mut(|mem| mem.request_focus(text_edit_id));
                }

                // Show mention autocomplete dropdown as floating popup above the input
                if self.mention_active && !matching_members.is_empty() {
                    // Use the horizontal row's rect for positioning (screen coordinates)
                    let input_rect = input_row_response.response.rect;
                    // Position at top of input, with bottom-left anchor so dropdown sits above
                    let dropdown_pos = egui::pos2(input_rect.left(), input_rect.top());

                    egui::Area::new(egui::Id::new("mention_dropdown"))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::LEFT_BOTTOM)  // Anchor at bottom-left
                        .fixed_pos(dropdown_pos)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::none()
                                .fill(super::theme::BG_ELEVATED)
                                .rounding(4.0)
                                .inner_margin(4.0)
                                .stroke(egui::Stroke::new(1.0, super::theme::BG_ACCENT))
                                .shadow(egui::epaint::Shadow {
                                    offset: egui::vec2(0.0, 2.0),
                                    blur: 8.0,
                                    spread: 0.0,
                                    color: egui::Color32::from_black_alpha(60),
                                })
                                .show(ui, |ui| {
                                    ui.set_min_width(250.0);
                                    for (i, (_, username, display_name)) in matching_members.iter().enumerate() {
                                        let is_selected = i == self.mention_selected;
                                        let text = if username != display_name {
                                            format!("{} ({})", display_name, username)
                                        } else {
                                            display_name.clone()
                                        };

                                        let response = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new(&text)
                                                    .color(if is_selected {
                                                        super::theme::TEXT_BRIGHT
                                                    } else {
                                                        super::theme::TEXT_NORMAL
                                                    })
                                            )
                                            .fill(if is_selected {
                                                super::theme::BG_ACCENT
                                            } else {
                                                egui::Color32::TRANSPARENT
                                            })
                                            .min_size(egui::vec2(242.0, 28.0))
                                        );

                                        if response.clicked() {
                                            self.insert_mention(username);
                                        }
                                    }
                                });
                        });
                }

                // GIF picker popup (above input)
                if self.gif_picker.is_open() {
                    let input_rect = input_row_response.response.rect;
                    if let Some(gif_url) = self.gif_picker.show(
                        ui.ctx(),
                        input_rect,
                        network,
                        state,
                        runtime,
                    ) {
                        // Insert selected GIF URL into message
                        if !self.message_input.is_empty() && !self.message_input.ends_with(' ') && !self.message_input.ends_with('\n') {
                            self.message_input.push(' ');
                        }
                        self.message_input.push_str(&gif_url);
                        self.gif_picker.close();
                    }
                }

                ui.add_space(4.0);

                // Send typing indicator when user is typing
                let current_len = self.message_input.len();
                if current_len > self.prev_input_len && current_len > 0 {
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

        // Pinned messages panel (right side)
        if self.show_pinned_panel {
            egui::SidePanel::right("pinned_messages_panel")
                .default_width(320.0)
                .min_width(280.0)
                .max_width(400.0)
                .resizable(true)
                .show_inside(ui, |ui| {
                    ui.vertical(|ui| {
                        // Panel header
                        ui.horizontal(|ui| {
                            ui.heading("📌 Pinned Messages");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("✕").clicked() {
                                    self.show_pinned_panel = false;
                                }
                            });
                        });
                        ui.separator();

                        // Pinned messages list
                        if self.pinned_messages_loading {
                            ui.centered_and_justified(|ui| {
                                ui.spinner();
                            });
                        } else if self.pinned_messages.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new("No pinned messages")
                                        .color(egui::Color32::GRAY)
                                        .italics()
                                );
                            });
                        } else {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for pinned_msg in &self.pinned_messages {
                                        ui.group(|ui| {
                                            // Author and time
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&pinned_msg.author_name)
                                                        .strong()
                                                        .color(egui::Color32::from_rgb(200, 200, 255))
                                                );
                                                ui.label(
                                                    egui::RichText::new(format_relative_time(pinned_msg.created_at))
                                                        .small()
                                                        .color(egui::Color32::GRAY)
                                                );
                                            });

                                            // Message content (truncated if long)
                                            let content = if pinned_msg.content.len() > 200 {
                                                format!("{}...", &pinned_msg.content[..200])
                                            } else {
                                                pinned_msg.content.clone()
                                            };
                                            ui.label(&content);

                                            // Pinned by info
                                            if let Some(ref pinned_by) = pinned_msg.pinned_by {
                                                ui.label(
                                                    egui::RichText::new(format!("Pinned by {}", pinned_by))
                                                        .small()
                                                        .italics()
                                                        .color(egui::Color32::from_rgb(140, 140, 140))
                                                );
                                            }

                                            // Jump to message button
                                            if ui.small_button("Jump to message").clicked() {
                                                // Set the scroll target to this message
                                                let state = state.clone();
                                                let msg_id = pinned_msg.id;
                                                runtime.spawn(async move {
                                                    let mut s = state.write().await;
                                                    s.scroll_to_message_id = Some(msg_id);
                                                });
                                            }
                                        });
                                        ui.add_space(4.0);
                                    }
                                });
                        }
                    });
                });
        }

        // Messages area fills remaining space
        egui::CentralPanel::default()
            .show_inside(ui, |ui| {
                // Build a lookup map for reply previews
                let message_lookup: HashMap<Uuid, &MessageData> = messages.iter().map(|m| (m.id, m)).collect();

                // Options for chat messages
                let options = MessageRenderOptions {
                    show_thread_button: true,
                    show_thread_indicator: true,
                    show_reply_button: true,
                    id_prefix: "chat",
                };

                // Only stick to bottom if we're not scrolling to a specific message and not loading history
                let should_stick_to_bottom = scroll_to_message_id.is_none()
                    && !self.loading_history
                    && self.scrolled_to_bottom
                    && !self.jump_to_bottom_requested;

                // Handle jump to bottom request
                let force_scroll_to_bottom = self.jump_to_bottom_requested;
                if force_scroll_to_bottom {
                    self.jump_to_bottom_requested = false;
                    self.scrolled_to_bottom = true;
                }

                let scroll_output = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(should_stick_to_bottom || force_scroll_to_bottom)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        // Show loading indicator at top when loading history
                        if self.loading_history {
                            ui.horizontal(|ui| {
                                ui.add_space(ui.available_width() / 2.0 - 50.0);
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Loading older messages...")
                                        .small()
                                        .color(egui::Color32::from_rgb(140, 140, 140)),
                                );
                            });
                            ui.add_space(8.0);
                        } else if !self.reached_history_end && !messages.is_empty() {
                            // Show "Load more" hint at top
                            ui.horizontal(|ui| {
                                ui.add_space(ui.available_width() / 2.0 - 40.0);
                                ui.label(
                                    egui::RichText::new("↑ Scroll up for more")
                                        .small()
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                );
                            });
                            ui.add_space(8.0);
                        }

                        let mut prev_message: Option<&MessageData> = None;
                        let mut found_scroll_target = false;

                        // Messages come from server in DESC order (newest first)
                        // Reverse to show oldest first (newest at bottom)
                        for message in messages.iter().rev() {
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
                                    let preview_content: String = original_msg.content.chars().take(100).collect();
                                    let preview_content = if original_msg.content.len() > 100 {
                                        format!("{}...", preview_content)
                                    } else {
                                        preview_content
                                    };

                                    ui.horizontal(|ui| {
                                        ui.add_space(16.0);
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

                            // Get reactions for this message from state
                            let reactions = message_reactions.get(&message.id).map(|v| v.as_slice());

                            // Check if this is the message we want to scroll to
                            let is_scroll_target = scroll_to_message_id == Some(message.id);

                            // Check if this message is selected via keyboard navigation
                            let is_selected = self.selected_message_id == Some(message.id);

                            // Track the position before rendering for scroll target
                            let before_cursor = ui.cursor();

                            // Add highlight background if this is the scroll target
                            if is_scroll_target {
                                let highlight_color = egui::Color32::from_rgba_unmultiplied(88, 101, 242, 40);
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        before_cursor.min,
                                        egui::vec2(ui.available_width(), 80.0),
                                    ),
                                    egui::Rounding::same(4.0),
                                    highlight_color,
                                );
                            }

                            // Add selection highlight for keyboard navigation
                            if is_selected {
                                let selection_color = egui::Color32::from_rgba_unmultiplied(100, 120, 180, 50);
                                let selection_border = egui::Color32::from_rgb(88, 101, 242);
                                let selection_rect = egui::Rect::from_min_size(
                                    before_cursor.min,
                                    egui::vec2(ui.available_width(), 80.0),
                                );
                                ui.painter().rect_filled(
                                    selection_rect,
                                    egui::Rounding::same(4.0),
                                    selection_color,
                                );
                                // Add left border to indicate selection
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        before_cursor.min,
                                        egui::vec2(3.0, 80.0),
                                    ),
                                    egui::Rounding::same(2.0),
                                    selection_border,
                                );
                            }

                            // Render the message using the shared component
                            if let Some(action) = render_message(
                                ui,
                                message,
                                current_user_id,
                                reactions,
                                state,
                                network,
                                runtime,
                                &mut self.renderer_state,
                                &options,
                            ) {
                                match action {
                                    MessageAction::Reply(msg) => {
                                        self.replying_to = Some(msg);
                                        self.editing_message = None;
                                    }
                                    MessageAction::Edit(msg) => {
                                        self.editing_message = Some(msg.clone());
                                        self.message_input = msg.content.clone();
                                        self.replying_to = None;
                                    }
                                    MessageAction::OpenThread(msg_id) => {
                                        let state = state.clone();
                                        runtime.spawn(async move {
                                            state.open_thread(msg_id).await;
                                        });
                                    }
                                    MessageAction::Pin(msg_id) => {
                                        let network = network.clone();
                                        runtime.spawn(async move {
                                            if let Err(e) = network.pin_message(msg_id).await {
                                                tracing::warn!("Failed to pin message: {}", e);
                                            }
                                        });
                                    }
                                    MessageAction::Unpin(msg_id) => {
                                        let network = network.clone();
                                        runtime.spawn(async move {
                                            if let Err(e) = network.unpin_message(msg_id).await {
                                                tracing::warn!("Failed to unpin message: {}", e);
                                            }
                                        });
                                    }
                                }
                            }

                            // Scroll to this message if it's the target
                            if is_scroll_target && !found_scroll_target {
                                found_scroll_target = true;
                                // Get the rect after rendering and scroll to it
                                let after_cursor = ui.cursor();
                                let message_rect = egui::Rect::from_min_max(
                                    before_cursor.min,
                                    egui::pos2(before_cursor.min.x + ui.available_width(), after_cursor.min.y),
                                );
                                ui.scroll_to_rect(message_rect, Some(egui::Align::Center));

                                // Clear the scroll target after a short delay to allow re-render
                                let state = state.clone();
                                runtime.spawn(async move {
                                    // Small delay to ensure the scroll completes
                                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                    let mut s = state.write().await;
                                    s.scroll_to_message_id = None;
                                });
                            }

                            // Scroll to keep selected message visible during keyboard navigation
                            if is_selected {
                                let after_cursor = ui.cursor();
                                let message_rect = egui::Rect::from_min_max(
                                    before_cursor.min,
                                    egui::pos2(before_cursor.min.x + ui.available_width(), after_cursor.min.y),
                                );
                                // Use MIN align to gently scroll just enough to show the message
                                ui.scroll_to_rect(message_rect, Some(egui::Align::Min));
                            }

                            ui.add_space(8.0);
                            prev_message = Some(message);
                        }
                    });

                // Check scroll position for infinite scroll
                let scroll_offset = scroll_output.state.offset.y;
                let content_height = scroll_output.content_size.y;
                let viewport_height = scroll_output.inner_rect.height();

                // Update scrolled_to_bottom state
                let at_bottom = content_height <= viewport_height ||
                    scroll_offset >= content_height - viewport_height - 50.0;
                self.scrolled_to_bottom = at_bottom;

                // Detect when loading completes by checking message count changes
                let current_message_count = messages.len();
                if self.loading_history {
                    if current_message_count > self.last_message_count {
                        // New messages were added - loading is complete
                        self.loading_history = false;
                        self.loading_started_at = None;
                    } else if let Some(started) = self.loading_started_at {
                        // Check for timeout (2 seconds) - assume we've reached the end
                        if started.elapsed().as_secs() >= 2 {
                            self.loading_history = false;
                            self.loading_started_at = None;
                            self.reached_history_end = true;
                        }
                    }
                }
                self.last_message_count = current_message_count;

                // Load more messages when scrolled near the top
                let near_top = scroll_offset < 100.0;
                if near_top
                    && !self.loading_history
                    && !self.reached_history_end
                    && !messages.is_empty()
                    && current_channel.is_some()
                {
                    self.loading_history = true;
                    self.loading_started_at = Some(Instant::now());
                    // Get the oldest message ID (first in the list since we store DESC)
                    if let Some(oldest_msg) = messages.first() {
                        let oldest_id = oldest_msg.id;
                        let channel_id = current_channel.unwrap();
                        let state = state.clone();
                        let network = network.clone();

                        runtime.spawn(async move {
                            match network.get_messages(channel_id, Some(oldest_id)).await {
                                Ok(older_messages) => {
                                    // Append older messages to the existing list
                                    let mut s = state.write().await;
                                    if let Some(existing) = s.messages.get_mut(&channel_id) {
                                        // Messages are in DESC order, older messages go after existing
                                        existing.extend(older_messages);
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to load older messages: {}", e);
                                }
                            }
                        });
                    }
                }

                // Show "Jump to present" button when not at bottom
                if !at_bottom {
                    let button_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            scroll_output.inner_rect.center().x,
                            scroll_output.inner_rect.bottom() - 40.0,
                        ),
                        egui::vec2(140.0, 32.0),
                    );

                    let painter = ui.painter();
                    painter.rect_filled(
                        button_rect,
                        egui::Rounding::same(16.0),
                        egui::Color32::from_rgb(88, 101, 242),
                    );

                    let response = ui.put(
                        button_rect,
                        egui::Button::new(
                            egui::RichText::new("↓ Jump to present")
                                .color(egui::Color32::WHITE)
                                .size(13.0),
                        )
                        .fill(egui::Color32::from_rgb(88, 101, 242))
                        .rounding(egui::Rounding::same(16.0)),
                    );

                    if response.clicked() {
                        self.jump_to_bottom_requested = true;
                    }
                }
            });

        // Render lightbox overlay on top if an image is being viewed
        render_lightbox(ui.ctx(), &mut self.renderer_state);
    }

    fn send_message(
        &mut self,
        channel_id: uuid::Uuid,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        // Allow sending if there's text OR attachments
        if self.message_input.trim().is_empty() && self.pending_attachments.is_empty() {
            return;
        }

        let content = self.message_input.clone();
        self.message_input.clear();

        // Clear the draft since we're sending
        let state_for_draft = state.clone();
        runtime.spawn(async move {
            state_for_draft.clear_draft(channel_id).await;
        });

        // Take pending attachments
        let attachments: Vec<(String, String, Vec<u8>)> = self.pending_attachments
            .drain(..)
            .map(|a| (a.filename, a.content_type, a.data))
            .collect();

        // Reset typing state
        self.last_typing_sent = None;
        self.prev_input_len = 0;

        let network = network.clone();

        // Check if we're editing or replying
        if let Some(edit_msg) = self.editing_message.take() {
            // Can't add attachments when editing
            runtime.spawn(async move {
                network.stop_typing(channel_id).await;
                if let Err(e) = network.update_message(edit_msg.id, &content).await {
                    tracing::warn!("Failed to update message: {}", e);
                }
            });
        } else {
            let reply_to_id = self.replying_to.take().map(|m| m.id);
            runtime.spawn(async move {
                network.stop_typing(channel_id).await;

                // Upload attachments first if any, collect their IDs
                let mut attachment_ids = Vec::new();
                if !attachments.is_empty() {
                    match network.upload_files(channel_id, attachments).await {
                        Ok(uploaded) => {
                            attachment_ids = uploaded.iter().map(|a| a.id).collect();
                        }
                        Err(e) => {
                            tracing::warn!("Failed to upload attachments: {}", e);
                        }
                    }
                }

                // Send the message with attachments (or just attachments if no text)
                let has_content = !content.trim().is_empty();
                let has_attachments = !attachment_ids.is_empty();

                if has_content || has_attachments {
                    // Use empty content if only attachments
                    let msg_content = if has_content { &content } else { "" };
                    let _ = network.send_message_with_attachments(
                        channel_id,
                        msg_content,
                        reply_to_id,
                        attachment_ids,
                    ).await;
                }
            });
        }
    }

    /// Open a file picker dialog to select files to attach
    fn open_file_picker(&mut self) {
        // Use rfd for file dialog
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Select files to attach")
            .pick_files()
        {
            for path in paths {
                self.add_file_from_path(&path);
            }
        }
    }

    /// Handle dropped files (drag-and-drop)
    fn handle_dropped_files(&mut self, ui: &mut egui::Ui) {
        // Check for dropped files
        let dropped_files = ui.ctx().input(|i| i.raw.dropped_files.clone());

        for file in dropped_files {
            if let Some(path) = &file.path {
                self.add_file_from_path(path);
            } else if let Some(bytes) = &file.bytes {
                // File dropped from certain sources may only have bytes, not path
                let filename = file.name.clone();
                let data = bytes.to_vec();

                // Check size limit (25MB)
                const MAX_SIZE: usize = 25 * 1024 * 1024;
                if data.len() > MAX_SIZE {
                    tracing::warn!("File {} is too large (max 25MB)", filename);
                    continue;
                }

                // Determine content type from filename extension
                let content_type = std::path::Path::new(&filename)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| Self::content_type_from_extension(ext))
                    .unwrap_or("application/octet-stream")
                    .to_string();

                self.pending_attachments.push(PendingAttachment {
                    filename,
                    content_type,
                    data,
                });
            }
        }

        // Visual feedback for drag-over
        let is_being_dragged = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if is_being_dragged {
            // Show overlay
            let screen_rect = ui.ctx().screen_rect();
            ui.painter().rect_filled(
                screen_rect,
                egui::Rounding::ZERO,
                egui::Color32::from_rgba_unmultiplied(88, 101, 242, 100), // Semi-transparent blue
            );
            ui.painter().text(
                screen_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop files to attach",
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
        }
    }

    /// Add a file from a path
    fn add_file_from_path(&mut self, path: &std::path::Path) {
        if let Some(filename) = path.file_name() {
            let filename = filename.to_string_lossy().to_string();

            // Read file
            match std::fs::read(path) {
                Ok(data) => {
                    // Check size limit (25MB)
                    const MAX_SIZE: usize = 25 * 1024 * 1024;
                    if data.len() > MAX_SIZE {
                        tracing::warn!("File {} is too large (max 25MB)", filename);
                        return;
                    }

                    // Determine content type from extension
                    let content_type = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|ext| Self::content_type_from_extension(ext))
                        .unwrap_or("application/octet-stream")
                        .to_string();

                    self.pending_attachments.push(PendingAttachment {
                        filename,
                        content_type,
                        data,
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to read file {}: {}", filename, e);
                }
            }
        }
    }

    /// Get content type from file extension
    fn content_type_from_extension(ext: &str) -> &'static str {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "txt" => "text/plain",
            "mp3" => "audio/mpeg",
            "mp4" => "video/mp4",
            "mov" => "video/quicktime",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        }
    }

    /// Insert formatting markers around cursor position or at end
    fn insert_formatting(&mut self, prefix: &str, suffix: &str) {
        // For simplicity, just append at end since egui doesn't give us cursor position easily
        // A more sophisticated implementation would track cursor position
        if self.message_input.is_empty() {
            self.message_input = format!("{}{}", prefix, suffix);
        } else {
            // Add a space if the last char isn't whitespace
            if !self.message_input.ends_with(' ') && !self.message_input.ends_with('\n') {
                self.message_input.push(' ');
            }
            self.message_input.push_str(prefix);
            self.message_input.push_str(suffix);
        }
    }

    /// Update mention autocomplete state based on input
    fn update_mention_state(&mut self) {
        // Find the last @ in the input
        if let Some(at_pos) = self.message_input.rfind('@') {
            let after_at = &self.message_input[at_pos + 1..];

            // Check if there's a space after @ (mention complete) or no @ at all
            if after_at.contains(' ') || after_at.contains('\n') {
                self.mention_active = false;
                self.mention_query.clear();
                self.mention_selected = 0;
                self.mention_dismissed = false;  // Reset dismissed when mention is completed
            } else {
                // Active mention - extract query
                let new_query = after_at.to_string();
                // Only reset selection when query actually changes
                if new_query != self.mention_query {
                    self.mention_selected = 0;
                    // If query changed, user typed something new - clear dismissed state
                    if self.mention_dismissed {
                        self.mention_dismissed = false;
                    }
                }
                // Only activate if not dismissed
                if !self.mention_dismissed {
                    self.mention_active = true;
                }
                self.mention_query = new_query;
            }
        } else {
            self.mention_active = false;
            self.mention_query.clear();
            self.mention_selected = 0;
            self.mention_dismissed = false;  // Reset dismissed when @ is removed
        }
    }

    /// Insert a mention by replacing the @query with @username
    fn insert_mention(&mut self, username: &str) {
        if let Some(at_pos) = self.message_input.rfind('@') {
            // Replace @query with @username
            self.message_input.truncate(at_pos);
            let mention = format!("@{} ", username);
            self.message_input.push_str(&mention);
            // Set cursor to end of the inserted mention
            self.pending_cursor_pos = Some(self.message_input.chars().count());
        }
        self.mention_active = false;
        self.mention_query.clear();
        self.mention_selected = 0;
    }

    /// Handle keyboard navigation for messages (Arrow keys, E, R, Delete, Escape)
    fn handle_keyboard_navigation(
        &mut self,
        ui: &mut egui::Ui,
        messages: &[MessageData],
        current_user_id: Option<Uuid>,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        // Don't handle navigation if we're editing or the input is focused
        // We check if any text edit has focus
        if self.editing_message.is_some() || self.mention_active {
            return;
        }

        // Get message IDs in display order (reversed from storage, oldest first)
        let message_ids: Vec<Uuid> = messages.iter().rev().map(|m| m.id).collect();

        if message_ids.is_empty() {
            return;
        }

        // Get current selection index
        let current_index = self.selected_message_id
            .and_then(|id| message_ids.iter().position(|&mid| mid == id));

        // Handle keyboard input
        ui.ctx().input_mut(|i| {
            // Escape - deselect
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.selected_message_id = None;
                self.replying_to = None;
                return;
            }

            // Only process navigation keys if we have a selection or want to start one
            // ArrowUp - move selection up (toward older messages / toward top)
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                if let Some(idx) = current_index {
                    if idx > 0 {
                        self.selected_message_id = Some(message_ids[idx - 1]);
                    }
                } else {
                    // No selection, select the newest message (last in display order)
                    self.selected_message_id = message_ids.last().copied();
                }
                return;
            }

            // ArrowDown - move selection down (toward newer messages / toward bottom)
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                if let Some(idx) = current_index {
                    if idx < message_ids.len() - 1 {
                        self.selected_message_id = Some(message_ids[idx + 1]);
                    }
                } else {
                    // No selection, select the newest message
                    self.selected_message_id = message_ids.last().copied();
                }
                return;
            }

            // Only handle action keys if we have a selection
            if let Some(selected_id) = self.selected_message_id {
                // Find the selected message
                if let Some(msg) = messages.iter().find(|m| m.id == selected_id) {
                    // E - Edit (only own messages)
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::E) {
                        if current_user_id == Some(msg.author_id) {
                            self.editing_message = Some(msg.clone());
                            self.message_input = msg.content.clone();
                            self.replying_to = None;
                            self.selected_message_id = None;
                        }
                        return;
                    }

                    // R - Reply
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::R) {
                        self.replying_to = Some(msg.clone());
                        self.editing_message = None;
                        self.selected_message_id = None;
                        return;
                    }

                    // Delete - Delete message (only own messages)
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                    {
                        if current_user_id == Some(msg.author_id) {
                            let network = network.clone();
                            let msg_id = msg.id;
                            let state = state.clone();
                            let channel_id = msg.channel_id;
                            runtime.spawn(async move {
                                if let Err(e) = network.delete_message(msg_id).await {
                                    tracing::warn!("Failed to delete message: {}", e);
                                } else {
                                    // Remove from local state
                                    let mut s = state.write().await;
                                    if let Some(msgs) = s.messages.get_mut(&channel_id) {
                                        msgs.retain(|m| m.id != msg_id);
                                    }
                                }
                            });
                            self.selected_message_id = None;
                        }
                        return;
                    }
                }
            }
        });
    }
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}
