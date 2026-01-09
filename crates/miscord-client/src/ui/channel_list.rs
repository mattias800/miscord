use eframe::egui;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::{AppState, VoiceParticipant};
use miscord_protocol::ChannelType;

use super::theme;

/// How often to refresh voice participants for channels we're not in (in seconds)
const VOICE_PARTICIPANTS_REFRESH_INTERVAL: f32 = 1.0;

pub struct ChannelList {
    show_create_dialog: bool,
    new_channel_name: String,
    new_channel_type: ChannelType,
    show_invite_dialog: bool,
    invite_code: Option<String>,
    invite_loading: bool,
    /// Cache of voice participants for channels we're not in
    voice_participants_cache: HashMap<Uuid, Vec<VoiceParticipant>>,
    /// Last time we fetched voice participants
    voice_participants_last_fetch: Option<Instant>,
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
            voice_participants_cache: HashMap::new(),
            voice_participants_last_fetch: None,
        }
    }

    /// Show the channel list. Returns (text_channels_expanded, voice_channels_expanded).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
        text_channels_expanded: bool,
        voice_channels_expanded: bool,
    ) -> (bool, bool) {
        let mut text_expanded = text_channels_expanded;
        let mut voice_expanded = voice_channels_expanded;
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
            return (text_expanded, voice_expanded);
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
                let text_response = egui::CollapsingHeader::new("Text Channels")
                    .default_open(text_expanded)
                    .show(ui, |ui| {
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

                                    // Populate message_reactions from loaded messages
                                    for msg in &messages {
                                        if !msg.reactions.is_empty() {
                                            let mut emoji_reactions: std::collections::HashMap<String, crate::state::ReactionState> = std::collections::HashMap::new();
                                            for reaction in &msg.reactions {
                                                let reaction_state = crate::state::ReactionState {
                                                    user_ids: reaction.user_ids.iter().copied().collect(),
                                                };
                                                emoji_reactions.insert(reaction.emoji.clone(), reaction_state);
                                            }
                                            s.message_reactions.insert(msg.id, emoji_reactions);
                                        }
                                    }

                                    s.messages.insert(channel_id, messages);
                                }

                                // Subscribe to channel
                                network.subscribe_channel(channel_id).await;
                            });
                        }
                    }
                });
                text_expanded = text_response.fully_open();
            }

            ui.add_space(8.0);

            // Voice channels
            let voice_channels: Vec<_> = channels
                .iter()
                .filter(|c| matches!(c.channel_type, ChannelType::Voice))
                .collect();

            if !voice_channels.is_empty() {
                // Check if we need to refresh the voice participants cache
                let should_refresh = self.voice_participants_last_fetch
                    .map(|t| t.elapsed().as_secs_f32() > VOICE_PARTICIPANTS_REFRESH_INTERVAL)
                    .unwrap_or(true);

                if should_refresh {
                    // Fetch participants for all voice channels we're not in
                    let voice_channel_ids: Vec<_> = voice_channels.iter().map(|c| c.id).collect();
                    let current_voice_channel = runtime.block_on(async {
                        state.read().await.voice_channel_id
                    });

                    for channel_id in voice_channel_ids {
                        if current_voice_channel != Some(channel_id) {
                            let participants = runtime.block_on(async {
                                match network.get_voice_participants(channel_id).await {
                                    Ok(server_participants) => {
                                        server_participants.into_iter().map(|p| {
                                            VoiceParticipant {
                                                user_id: p.user_id,
                                                username: p.username,
                                                is_muted: p.self_muted,
                                                is_deafened: p.self_deafened,
                                                is_video_enabled: p.video_enabled,
                                                is_screen_sharing: p.screen_sharing,
                                                is_speaking: false,
                                                speaking_since: None,
                                            }
                                        }).collect()
                                    }
                                    Err(_) => Vec::new()
                                }
                            });
                            self.voice_participants_cache.insert(channel_id, participants);
                        }
                    }
                    self.voice_participants_last_fetch = Some(Instant::now());
                }

                let voice_response = egui::CollapsingHeader::new("Voice Channels")
                    .default_open(voice_expanded)
                    .show(ui, |ui| {
                    for channel in voice_channels {
                        // Get participants - from local state if we're in the channel, from cache otherwise
                        let (voice_channel_id, participants, local_speaking) = runtime.block_on(async {
                            let s = state.read().await;
                            let voice_channel_id = s.voice_channel_id;
                            let local_speaking = s.is_speaking;

                            let participants = if voice_channel_id == Some(channel.id) {
                                // We're in this channel - use local state
                                s.voice_participants.values().cloned().collect()
                            } else {
                                // Use cached data
                                Vec::new() // Will be filled from cache below
                            };

                            (voice_channel_id, participants, local_speaking)
                        });

                        // If not in this channel, use cached participants
                        let participants = if voice_channel_id != Some(channel.id) {
                            self.voice_participants_cache.get(&channel.id).cloned().unwrap_or_default()
                        } else {
                            participants
                        };

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
                                    // Subscribe FIRST so we receive broadcasts about other users joining
                                    network.subscribe_channel(channel_id).await;

                                    // Set local voice channel BEFORE API call so we're ready
                                    // to receive VoiceUserJoined broadcasts (including our own)
                                    state.join_voice(channel_id).await;

                                    if network.join_voice(channel_id).await.is_ok() {
                                        // Fetch existing participants and add them
                                        if let Ok(existing) = network.get_voice_participants(channel_id).await {
                                            let mut s = state.write().await;
                                            for p in existing {
                                                // Don't overwrite ourselves
                                                if s.current_user.as_ref().map(|u| u.id) != Some(p.user_id) {
                                                    s.voice_participants.insert(p.user_id, crate::state::VoiceParticipant {
                                                        user_id: p.user_id,
                                                        username: p.username,
                                                        is_muted: p.self_muted,
                                                        is_deafened: p.self_deafened,
                                                        is_video_enabled: p.video_enabled,
                                                        is_screen_sharing: p.screen_sharing,
                                                        is_speaking: false,
                                                        speaking_since: None,
                                                    });
                                                }
                                            }
                                        }
                                    } else {
                                        // API call failed, revert local state
                                        state.leave_voice().await;
                                    }
                                });
                            }
                        }

                        // Show participants under the voice channel
                        if !participants.is_empty() {
                            let current_user_id = runtime.block_on(async {
                                state.read().await.current_user.as_ref().map(|u| u.id)
                            });

                            for participant in &participants {
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0); // Indent

                                    // Small avatar circle with initial
                                    let initial = participant.username
                                        .chars()
                                        .next()
                                        .unwrap_or('?')
                                        .to_uppercase()
                                        .to_string();

                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(16.0, 16.0),
                                        egui::Sense::hover(),
                                    );
                                    let painter = ui.painter_at(rect);
                                    painter.circle_filled(rect.center(), 8.0, theme::BG_ACCENT);
                                    painter.text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        &initial,
                                        egui::FontId::proportional(9.0),
                                        theme::TEXT_NORMAL,
                                    );

                                    ui.add_space(4.0);

                                    // Check if this participant is speaking
                                    let is_self = current_user_id == Some(participant.user_id);
                                    let is_speaking = if is_self {
                                        local_speaking
                                    } else {
                                        participant.is_speaking
                                    };

                                    // Username - bright white when speaking, muted when silent
                                    let name_color = if is_speaking {
                                        theme::TEXT_NORMAL
                                    } else {
                                        theme::TEXT_MUTED
                                    };

                                    ui.label(
                                        egui::RichText::new(&participant.username)
                                            .color(name_color)
                                            .size(12.0),
                                    );

                                    // Status icons
                                    if participant.is_muted {
                                        ui.label(
                                            egui::RichText::new("🔇")
                                                .size(10.0)
                                                .color(theme::TEXT_MUTED),
                                        );
                                    }
                                    if participant.is_video_enabled {
                                        ui.label(
                                            egui::RichText::new("📹")
                                                .size(10.0)
                                                .color(theme::TEXT_MUTED),
                                        );
                                    }
                                    if participant.is_screen_sharing {
                                        ui.label(
                                            egui::RichText::new("🖥")
                                                .size(10.0)
                                                .color(theme::TEXT_MUTED),
                                        );
                                    }
                                });
                            }
                        }
                    }
                });
                voice_expanded = voice_response.fully_open();
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

        (text_expanded, voice_expanded)
    }
}

impl Default for ChannelList {
    fn default() -> Self {
        Self::new()
    }
}
