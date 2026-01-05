use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;
use miscord_protocol::ChannelType;

pub struct ChannelList {
    show_create_dialog: bool,
    new_channel_name: String,
    new_channel_type: ChannelType,
    show_invite_dialog: bool,
    invite_code: Option<String>,
    invite_loading: bool,
}

impl ChannelList {
    pub fn new() -> Self {
        Self {
            show_create_dialog: false,
            new_channel_name: String::new(),
            new_channel_type: ChannelType::Text,
            show_invite_dialog: false,
            invite_code: None,
            invite_loading: false,
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (current_community, channels, current_channel) = runtime.block_on(async {
            let s = state.read().await;
            let current_community = s.current_community_id;
            let channels: Vec<_> = s
                .channels
                .values()
                .filter(|c| c.community_id == current_community)
                .cloned()
                .collect();
            let current_channel = s.current_channel_id;
            (current_community, channels, current_channel)
        });

        if current_community.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Select a community");
            });
            return;
        }

        let community_id = current_community.unwrap();

        ui.vertical(|ui| {
            // Community name header
            let community_name = runtime.block_on(async {
                state
                    .read()
                    .await
                    .communities
                    .get(&community_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default()
            });

            ui.horizontal(|ui| {
                ui.heading(&community_name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("+").on_hover_text("Create Channel").clicked() {
                        self.show_create_dialog = true;
                    }
                    if ui.button("Invite").on_hover_text("Create Invite Link").clicked() {
                        self.show_invite_dialog = true;
                        self.invite_code = None;
                        self.invite_loading = true;

                        let network = network.clone();
                        let state = state.clone();

                        runtime.spawn(async move {
                            match network.create_invite(community_id).await {
                                Ok(code) => {
                                    let mut s = state.write().await;
                                    s.pending_invite_code = Some(code);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to create invite: {}", e);
                                }
                            }
                        });
                    }
                });
            });

            ui.separator();

            // Text channels
            let text_channels: Vec<_> = channels
                .iter()
                .filter(|c| matches!(c.channel_type, ChannelType::Text))
                .collect();

            if !text_channels.is_empty() {
                ui.collapsing("Text Channels", |ui| {
                    for channel in text_channels {
                        let is_selected = current_channel == Some(channel.id);

                        let response = ui.selectable_label(
                            is_selected,
                            format!("# {}", channel.name),
                        );

                        if response.clicked() {
                            let state = state.clone();
                            let network = network.clone();
                            let channel_id = channel.id;

                            runtime.spawn(async move {
                                state.select_channel(channel_id).await;

                                // Load messages
                                if let Ok(messages) = network.get_messages(channel_id, None).await {
                                    let mut s = state.write().await;
                                    s.messages.insert(channel_id, messages);
                                }

                                // Subscribe to channel
                                network.subscribe_channel(channel_id).await;
                            });
                        }
                    }
                });
            }

            ui.add_space(8.0);

            // Voice channels
            let voice_channels: Vec<_> = channels
                .iter()
                .filter(|c| matches!(c.channel_type, ChannelType::Voice))
                .collect();

            if !voice_channels.is_empty() {
                ui.collapsing("Voice Channels", |ui| {
                    for channel in voice_channels {
                        let voice_channel_id = runtime.block_on(async {
                            state.read().await.voice_channel_id
                        });

                        let is_connected = voice_channel_id == Some(channel.id);

                        let response = ui.horizontal(|ui| {
                            ui.selectable_label(is_connected, format!("🔊 {}", channel.name))
                        }).inner;

                        if response.clicked() {
                            let state = state.clone();
                            let network = network.clone();
                            let channel_id = channel.id;

                            if is_connected {
                                // Leave voice
                                runtime.spawn(async move {
                                    network.leave_voice().await;
                                    state.leave_voice().await;
                                });
                            } else {
                                // Join voice
                                runtime.spawn(async move {
                                    if network.join_voice(channel_id).await.is_ok() {
                                        state.join_voice(channel_id).await;
                                    }
                                });
                            }
                        }
                    }
                });
            }
        });

        // Create channel dialog
        if self.show_create_dialog {
            egui::Window::new("Create Channel")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.new_channel_name);
                    });

                    ui.horizontal(|ui| {
                        ui.label("Type:");
                        ui.selectable_value(&mut self.new_channel_type, ChannelType::Text, "Text");
                        ui.selectable_value(&mut self.new_channel_type, ChannelType::Voice, "Voice");
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            let name = self.new_channel_name.clone();
                            let channel_type = self.new_channel_type.clone();
                            let network = network.clone();
                            let state = state.clone();

                            runtime.spawn(async move {
                                if let Ok(channel) =
                                    network.create_channel(community_id, &name, channel_type).await
                                {
                                    let mut s = state.write().await;
                                    s.channels.insert(channel.id, channel);
                                }
                            });

                            self.new_channel_name.clear();
                            self.show_create_dialog = false;
                        }

                        if ui.button("Cancel").clicked() {
                            self.new_channel_name.clear();
                            self.show_create_dialog = false;
                        }
                    });
                });
        }

        // Invite dialog
        if self.show_invite_dialog {
            // Check if invite code is ready
            let pending_code = runtime.block_on(async {
                let mut s = state.write().await;
                s.pending_invite_code.take()
            });

            if let Some(code) = pending_code {
                self.invite_code = Some(code);
                self.invite_loading = false;
            }

            egui::Window::new("Invite People")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    if self.invite_loading {
                        ui.label("Creating invite code...");
                        ui.spinner();
                    } else if let Some(code) = &self.invite_code {
                        ui.label("Share this invite code:");
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.monospace(code);
                            if ui.button("Copy").clicked() {
                                ui.output_mut(|o| o.copied_text = code.clone());
                            }
                        });

                        ui.add_space(8.0);
                    } else {
                        ui.label("Failed to create invite code");
                    }

                    if ui.button("Close").clicked() {
                        self.show_invite_dialog = false;
                        self.invite_code = None;
                        self.invite_loading = false;
                    }
                });
        }
    }
}

impl Default for ChannelList {
    fn default() -> Self {
        Self::new()
    }
}
