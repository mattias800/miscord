use chrono::{DateTime, Datelike, Local, Utc};
use eframe::egui;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;
use miscord_protocol::MessageData;

use super::message::{
    render_message, MessageAction, MessageRenderOptions, MessageRendererState, ReactionInfo,
};

/// How often to send typing indicators (in seconds)
const TYPING_THROTTLE_SECS: u64 = 3;

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
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (current_channel, messages, channel_name, typing_usernames, current_user_id, message_reactions, members) = runtime.block_on(async {
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

            (channel_id, messages, channel_name, typing_usernames, current_user_id, message_reactions, members)
        });

        if current_channel.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select a channel to start chatting");
            });
            return;
        }

        let channel_id = current_channel.unwrap();

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
                        self.mention_query.clear();
                        self.mention_selected = 0;
                        mention_handled = true;
                    }
                }

                // Message input
                let text_edit_id = ui.make_persistent_id("chat_message_input");
                ui.horizontal(|ui| {
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

                // Show mention autocomplete dropdown as floating popup above the input
                if self.mention_active && !matching_members.is_empty() {
                    // Calculate position - above the input panel
                    let input_rect = ui.min_rect();
                    let dropdown_height = (matching_members.len() as f32 * 32.0) + 8.0;
                    let dropdown_pos = egui::pos2(
                        input_rect.left() + 8.0,
                        input_rect.top() - dropdown_height - 4.0,
                    );

                    egui::Area::new(egui::Id::new("mention_dropdown"))
                        .order(egui::Order::Foreground)
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

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        let mut prev_message: Option<&MessageData> = None;

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
                let _ = network.send_message_with_reply(channel_id, &content, reply_to_id).await;
            });
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
            } else {
                // Active mention - extract query
                let new_query = after_at.to_string();
                // Only reset selection when query actually changes
                if new_query != self.mention_query {
                    self.mention_selected = 0;
                }
                self.mention_active = true;
                self.mention_query = new_query;
            }
        } else {
            self.mention_active = false;
            self.mention_query.clear();
            self.mention_selected = 0;
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
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}
