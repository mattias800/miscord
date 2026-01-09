use eframe::egui;
use tokio::sync::mpsc;

use crate::media::audio::{AudioCapture, AudioPlayback};
use crate::network::NetworkClient;
use crate::state::{AppState, UiState};

use super::channel_list::ChannelList;
use super::chat::ChatView;
use super::community_list::CommunityList;
use super::member_list::MemberList;
use super::thread_panel::ThreadPanel;
use super::voice::VoicePanel;
use super::voice_channel_view::VoiceChannelView;

pub struct MainView {
    community_list: CommunityList,
    channel_list: ChannelList,
    chat_view: ChatView,
    member_list: MemberList,
    thread_panel: ThreadPanel,
    voice_panel: VoicePanel,
    voice_channel_view: VoiceChannelView,
    // Audio state for voice channels
    audio_capture: Option<AudioCapture>,
    audio_playback: Option<AudioPlayback>,
    audio_rx: Option<mpsc::Receiver<Vec<f32>>>,
    was_in_voice: bool,
    // Persistent UI state
    ui_state: UiState,
    /// Whether we've done the initial restore of community/channel
    initial_restore_done: bool,
    /// Track last known community/channel to detect changes for saving
    last_community_id: Option<uuid::Uuid>,
    last_channel_id: Option<uuid::Uuid>,
}

impl MainView {
    pub fn new() -> Self {
        let ui_state = UiState::load();
        Self {
            community_list: CommunityList::new(),
            channel_list: ChannelList::new(),
            chat_view: ChatView::new(),
            member_list: MemberList::new(),
            thread_panel: ThreadPanel::new(),
            voice_panel: VoicePanel::new(),
            voice_channel_view: VoiceChannelView::new(),
            audio_capture: None,
            audio_playback: None,
            audio_rx: None,
            was_in_voice: false,
            ui_state,
            initial_restore_done: false,
            last_community_id: None,
            last_channel_id: None,
        }
    }

    /// Returns true if settings should be opened
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> bool {
        let mut open_settings = false;

        // Check voice state and manage audio capture
        let (in_voice, has_community, selected_input_device, gate_threshold_db, current_community_id, current_channel_id, communities_loaded) = runtime.block_on(async {
            let s = state.read().await;
            (
                s.voice_channel_id.is_some(),
                s.current_community_id.is_some(),
                s.selected_input_device.clone(),
                s.gate_threshold_db,
                s.current_community_id,
                s.current_channel_id,
                !s.communities.is_empty(),
            )
        });

        // Restore saved UI state on first render after communities are loaded
        if !self.initial_restore_done && communities_loaded {
            self.initial_restore_done = true;

            // Determine which community to select: saved one if valid, otherwise first available
            let target_community_id = runtime.block_on(async {
                let s = state.read().await;
                if let Some(saved_id) = self.ui_state.current_community_id {
                    if s.communities.contains_key(&saved_id) {
                        return Some(saved_id);
                    }
                }
                // Fall back to first community
                s.communities.keys().next().copied()
            });

            if let Some(community_id) = target_community_id {
                let state_clone = state.clone();
                let network_clone = network.clone();
                let saved_channel_id = self.ui_state.current_channel_id;

                runtime.spawn(async move {
                    // Select the community
                    state_clone.select_community(community_id).await;

                    // Load channels for this community
                    if let Ok(channels) = network_clone.get_channels(community_id).await {
                        state_clone.set_channels(channels.clone()).await;

                        // Load members for this community
                        if let Ok(members) = network_clone.get_members(community_id).await {
                            state_clone.set_members(community_id, members).await;
                        }

                        // Determine which channel to select: saved one if valid, otherwise first text channel
                        let target_channel = saved_channel_id
                            .and_then(|id| channels.iter().find(|c| c.id == id))
                            .or_else(|| {
                                channels.iter().find(|c| matches!(c.channel_type, miscord_protocol::ChannelType::Text))
                            });

                        if let Some(channel) = target_channel {
                            let channel_id = channel.id;
                            state_clone.select_channel(channel_id).await;

                            // Load messages for the channel
                            if let Ok(messages) = network_clone.get_messages(channel_id, None).await {
                                let mut s = state_clone.write().await;

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
                            network_clone.subscribe_channel(channel_id).await;
                        }
                    }
                });
            }
        }

        // Track changes to community/channel and save UI state
        if current_community_id != self.last_community_id || current_channel_id != self.last_channel_id {
            self.last_community_id = current_community_id;
            self.last_channel_id = current_channel_id;
            self.ui_state.current_community_id = current_community_id;
            self.ui_state.current_channel_id = current_channel_id;
            self.ui_state.save();
        }

        // Handle voice state transitions
        if in_voice && !self.was_in_voice {
            // Joining voice - start audio capture
            tracing::info!("Joining voice - starting audio capture");
            let mut capture = AudioCapture::new();
            capture.set_gate_threshold_db(gate_threshold_db);

            // Try to start with selected device, fall back to default if not found
            let start_result = match capture.start(selected_input_device.as_deref()) {
                Ok(rx) => Ok(rx),
                Err(e) => {
                    tracing::warn!("Failed to start audio capture with saved device: {}. Trying default device.", e);
                    capture.start(None)
                }
            };

            match start_result {
                Ok(rx) => {
                    // Initialize VAD with the audio capture's level monitor
                    let level_monitor = capture.level_monitor();
                    self.voice_channel_view.init_vad(level_monitor, gate_threshold_db);

                    // Start playback for loopback (for testing)
                    // In real app, this would send over network
                    let mut playback = AudioPlayback::new();
                    let output_device = runtime.block_on(async {
                        state.read().await.selected_output_device.clone()
                    });

                    // Try selected output device, fall back to default
                    if let Err(e) = playback.start_with_device(output_device.as_deref(), rx) {
                        tracing::warn!("Failed to start audio playback with saved device: {}. Audio loopback disabled.", e);
                    }

                    self.audio_capture = Some(capture);
                    self.audio_playback = Some(playback);
                    tracing::info!("Audio capture started successfully");
                }
                Err(e) => {
                    tracing::error!("Failed to start audio capture: {}", e);
                }
            }
        } else if !in_voice && self.was_in_voice {
            // Leaving voice - stop audio capture
            tracing::info!("Leaving voice - stopping audio capture");
            if let Some(mut capture) = self.audio_capture.take() {
                capture.stop();
            }
            if let Some(mut playback) = self.audio_playback.take() {
                playback.stop();
            }
            self.voice_channel_view.cleanup();
        }
        self.was_in_voice = in_voice;

        // Left panel - Community list
        egui::SidePanel::left("community_panel")
            .exact_width(72.0)
            .show(ctx, |ui| {
                self.community_list.show(ui, state, network, runtime);
            });

        // Channel list panel
        egui::SidePanel::left("channel_panel")
            .min_width(200.0)
            .max_width(300.0)
            .show(ctx, |ui| {
                let (text_expanded, voice_expanded) = self.channel_list.show(
                    ui,
                    state,
                    network,
                    runtime,
                    self.ui_state.text_channels_expanded,
                    self.ui_state.voice_channels_expanded,
                );

                // Save expanded states if they changed
                if text_expanded != self.ui_state.text_channels_expanded
                    || voice_expanded != self.ui_state.voice_channels_expanded
                {
                    self.ui_state.text_channels_expanded = text_expanded;
                    self.ui_state.voice_channels_expanded = voice_expanded;
                    self.ui_state.save();
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    // Settings button row
                    ui.horizontal(|ui| {
                        if ui.button("\u{2699}").on_hover_text("Settings").clicked() {
                            open_settings = true;
                        }
                    });
                    ui.add_space(4.0);

                    self.voice_panel.show_controls(ui, state, network, runtime);
                });
            });

        // Check if thread is open
        let thread_open = runtime.block_on(async {
            state.read().await.open_thread.is_some()
        });

        // Right panel - Always show member list when in a community
        if has_community {
            egui::SidePanel::right("member_panel")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.member_list.show(ui, state, runtime);
                    });
                });
        }

        // Thread panel (between chat and member list)
        if thread_open && !in_voice {
            egui::SidePanel::right("thread_panel")
                .min_width(320.0)
                .max_width(450.0)
                .show(ctx, |ui| {
                    let should_close = self.thread_panel.show(ui, state, network, runtime);
                    if should_close {
                        let state = state.clone();
                        let network = network.clone();
                        self.thread_panel.cleanup(&network, runtime);
                        runtime.spawn(async move {
                            state.close_thread().await;
                        });
                    }
                });
        }

        // Main content area - show voice channel view or chat view
        egui::CentralPanel::default().show(ctx, |ui| {
            if in_voice {
                self.voice_channel_view.show(ui, ctx, state, network, runtime);
            } else {
                self.chat_view.show(ui, state, network, runtime);
            }
        });

        open_settings
    }
}

impl Default for MainView {
    fn default() -> Self {
        Self::new()
    }
}
