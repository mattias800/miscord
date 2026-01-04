use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;

pub struct ChatView {
    message_input: String,
}

impl ChatView {
    pub fn new() -> Self {
        Self {
            message_input: String::new(),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (current_channel, messages, channel_name) = runtime.block_on(async {
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
            (channel_id, messages, channel_name)
        });

        if current_channel.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select a channel to start chatting");
            });
            return;
        }

        let channel_id = current_channel.unwrap();

        ui.vertical(|ui| {
            // Channel header
            ui.horizontal(|ui| {
                ui.heading(format!("# {}", channel_name));
            });

            ui.separator();

            // Messages area
            let available_height = ui.available_height() - 60.0;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(available_height)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for message in messages.iter() {
                        ui.horizontal(|ui| {
                            // Get author name
                            let author_name = runtime.block_on(async {
                                state
                                    .read()
                                    .await
                                    .users
                                    .get(&message.author_id)
                                    .map(|u| u.display_name.clone())
                                    .unwrap_or_else(|| "Unknown".to_string())
                            });

                            ui.label(
                                egui::RichText::new(&author_name)
                                    .strong()
                                    .color(egui::Color32::from_rgb(88, 101, 242)),
                            );

                            ui.label(&message.content);

                            // Timestamp
                            let time = message.created_at.format("%H:%M").to_string();
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(time)
                                            .small()
                                            .color(egui::Color32::GRAY),
                                    );
                                },
                            );
                        });

                        ui.add_space(4.0);
                    }
                });

            ui.separator();

            // Message input
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.message_input)
                        .hint_text(format!("Message #{}", channel_name))
                        .desired_width(ui.available_width() - 60.0),
                );

                // Send on Enter
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.send_message(channel_id, state, network, runtime);
                }

                if ui.button("Send").clicked() {
                    self.send_message(channel_id, state, network, runtime);
                }
            });
        });
    }

    fn send_message(
        &mut self,
        channel_id: uuid::Uuid,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        if self.message_input.trim().is_empty() {
            return;
        }

        let content = self.message_input.clone();
        self.message_input.clear();

        let network = network.clone();
        let state = state.clone();

        runtime.spawn(async move {
            if let Ok(message) = network.send_message(channel_id, &content).await {
                state.add_message(message).await;
            }
        });
    }
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}
