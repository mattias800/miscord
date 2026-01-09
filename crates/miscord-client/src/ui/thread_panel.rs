use eframe::egui;
use std::time::Instant;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;

use super::message::{
    render_message, MessageAction, MessageRenderOptions, MessageRendererState,
};

pub struct ThreadPanel {
    message_input: String,
    /// Last time we sent a typing indicator
    last_typing_sent: Option<Instant>,
    /// Previous message input length (to detect changes)
    prev_input_len: usize,
    /// Thread loading state
    is_loading: bool,
    /// Currently subscribed thread (to track subscriptions)
    subscribed_thread: Option<Uuid>,
    /// Shared message renderer state
    renderer_state: MessageRendererState,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            message_input: String::new(),
            last_typing_sent: None,
            prev_input_len: 0,
            is_loading: false,
            subscribed_thread: None,
            renderer_state: MessageRendererState::new(),
        }
    }

    /// Show the thread panel. Returns true if the panel should be closed.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> bool {
        let mut should_close = false;

        // Get thread state
        let (open_thread, thread_messages, parent_message, current_user_id) =
            runtime.block_on(async {
                let s = state.read().await;
                let open_thread = s.open_thread;
                let thread_messages = open_thread
                    .map(|id| s.thread_messages.get(&id).cloned().unwrap_or_default())
                    .unwrap_or_default();

                // Find parent message from channel messages
                let parent_message = open_thread.and_then(|parent_id| {
                    s.messages
                        .values()
                        .flatten()
                        .find(|m| m.id == parent_id)
                        .cloned()
                });

                let current_user_id = s.current_user.as_ref().map(|u| u.id);

                (open_thread, thread_messages, parent_message, current_user_id)
            });

        let Some(parent_message_id) = open_thread else {
            return false;
        };

        // Handle subscription changes
        if self.subscribed_thread != Some(parent_message_id) {
            // Unsubscribe from old thread
            if let Some(old_thread) = self.subscribed_thread.take() {
                let network = network.clone();
                runtime.spawn(async move {
                    network.unsubscribe_thread(old_thread).await;
                });
            }

            // Subscribe to new thread and load messages
            self.subscribed_thread = Some(parent_message_id);
            self.is_loading = true;
            let network = network.clone();
            let state = state.clone();
            runtime.spawn(async move {
                network.subscribe_thread(parent_message_id).await;

                // Load thread data
                match network.get_thread(parent_message_id).await {
                    Ok(thread_data) => {
                        state
                            .set_thread_messages(parent_message_id, thread_data.replies)
                            .await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to load thread: {}", e);
                    }
                }
            });
            self.is_loading = false;
        }

        // Header with close button
        egui::TopBottomPanel::top("thread_header")
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Thread");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            should_close = true;
                        }
                    });
                });
            });

        // Input area at bottom
        egui::TopBottomPanel::bottom("thread_input")
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut self.message_input)
                            .hint_text("Reply in thread...")
                            .desired_width(ui.available_width() - 60.0)
                            .desired_rows(2)
                            .lock_focus(true),
                    );

                    // Handle Enter (send) vs Shift+Enter (new line)
                    if response.has_focus() {
                        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let shift_held = ui.input(|i| i.modifiers.shift);

                        if enter_pressed && !shift_held {
                            if self.message_input.ends_with('\n') {
                                self.message_input.pop();
                            }
                            self.send_reply(parent_message_id, network, runtime);
                        }
                    }

                    if ui.button("Reply").clicked() {
                        self.send_reply(parent_message_id, network, runtime);
                    }
                });
                ui.add_space(4.0);
            });

        // Thread content
        egui::CentralPanel::default()
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());

                        // Options for parent message (no thread button/indicator in thread view)
                        let parent_options = MessageRenderOptions {
                            show_thread_button: false,
                            show_thread_indicator: false,
                            show_reply_button: false, // No inline reply in threads
                            id_prefix: "thread_parent",
                        };

                        // Options for thread replies
                        let reply_options = MessageRenderOptions {
                            show_thread_button: false,
                            show_thread_indicator: false,
                            show_reply_button: false,
                            id_prefix: "thread_reply",
                        };

                        // Show parent message
                        if let Some(parent) = &parent_message {
                            render_message(
                                ui,
                                parent,
                                current_user_id,
                                None, // Use message.reactions from loaded data
                                state,
                                network,
                                runtime,
                                &mut self.renderer_state,
                                &parent_options,
                            );
                            ui.separator();
                            ui.add_space(8.0);

                            // Reply count
                            let reply_count = thread_messages.len();
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    reply_count,
                                    if reply_count == 1 { "reply" } else { "replies" }
                                ))
                                .small()
                                .color(egui::Color32::from_rgb(140, 140, 140)),
                            );
                            ui.add_space(8.0);
                        }

                        // Show thread replies
                        if thread_messages.is_empty() && !self.is_loading {
                            ui.label(
                                egui::RichText::new("No replies yet. Start the conversation!")
                                    .italics()
                                    .color(egui::Color32::from_rgb(140, 140, 140)),
                            );
                        } else {
                            for message in &thread_messages {
                                render_message(
                                    ui,
                                    message,
                                    current_user_id,
                                    None, // Use message.reactions from loaded data
                                    state,
                                    network,
                                    runtime,
                                    &mut self.renderer_state,
                                    &reply_options,
                                );
                                ui.add_space(8.0);
                            }
                        }
                    });
            });

        should_close
    }

    fn send_reply(
        &mut self,
        parent_message_id: Uuid,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        if self.message_input.trim().is_empty() {
            return;
        }

        let content = self.message_input.clone();
        self.message_input.clear();
        self.last_typing_sent = None;
        self.prev_input_len = 0;

        let network = network.clone();
        runtime.spawn(async move {
            if let Err(e) = network.send_thread_reply(parent_message_id, &content, None).await {
                tracing::error!("Failed to send thread reply: {}", e);
            }
        });
    }

    /// Clean up when thread panel is closed
    pub fn cleanup(&mut self, network: &NetworkClient, runtime: &tokio::runtime::Runtime) {
        if let Some(thread_id) = self.subscribed_thread.take() {
            let network = network.clone();
            runtime.spawn(async move {
                network.unsubscribe_thread(thread_id).await;
            });
        }
        self.message_input.clear();
        self.renderer_state.emoji_picker_open_for = None;
    }
}

impl Default for ThreadPanel {
    fn default() -> Self {
        Self::new()
    }
}
