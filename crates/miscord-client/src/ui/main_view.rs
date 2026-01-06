use eframe::egui;
use tokio::sync::mpsc;

use crate::media::audio::{AudioCapture, AudioPlayback};
use crate::network::NetworkClient;
use crate::state::AppState;

use super::channel_list::ChannelList;
use super::chat::ChatView;
use super::community_list::CommunityList;
use super::member_list::MemberList;
use super::voice::VoicePanel;
use super::voice_channel_view::VoiceChannelView;

pub struct MainView {
    community_list: CommunityList,
    channel_list: ChannelList,
    chat_view: ChatView,
    member_list: MemberList,
    voice_panel: VoicePanel,
    voice_channel_view: VoiceChannelView,
    // Audio state for voice channels
    audio_capture: Option<AudioCapture>,
    audio_playback: Option<AudioPlayback>,
    audio_rx: Option<mpsc::Receiver<Vec<f32>>>,
    was_in_voice: bool,
}

impl MainView {
    pub fn new() -> Self {
        Self {
            community_list: CommunityList::new(),
            channel_list: ChannelList::new(),
            chat_view: ChatView::new(),
            member_list: MemberList::new(),
            voice_panel: VoicePanel::new(),
            voice_channel_view: VoiceChannelView::new(),
            audio_capture: None,
            audio_playback: None,
            audio_rx: None,
            was_in_voice: false,
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
        let (in_voice, has_community, selected_input_device, gate_threshold_db) = runtime.block_on(async {
            let s = state.read().await;
            (
                s.voice_channel_id.is_some(),
                s.current_community_id.is_some(),
                s.selected_input_device.clone(),
                s.gate_threshold_db,
            )
        });

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
                self.channel_list.show(ui, state, network, runtime);

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
