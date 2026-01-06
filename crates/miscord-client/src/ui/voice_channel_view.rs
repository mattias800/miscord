//! Voice Channel View component
//!
//! Displays a Discord-like grid of participants when in a voice channel.
//! Integrates with SFU (Selective Forwarding Unit) for video streaming.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::media::gst_video::GstVideoCapture;
use crate::media::sfu_client::SfuClient;
use crate::media::VoiceActivityDetector;
use crate::network::NetworkClient;
use crate::state::{AppState, VoiceParticipant};

use super::theme;

/// Voice channel view showing participant grid
pub struct VoiceChannelView {
    /// Local video capture
    video_capture: Option<GstVideoCapture>,
    /// Local video texture
    video_texture: Option<TextureHandle>,
    /// Voice Activity Detector
    vad: Option<VoiceActivityDetector>,
    /// Remote video textures (user_id -> texture)
    remote_textures: HashMap<Uuid, TextureHandle>,
    /// SFU client for video streaming
    sfu_client: Option<SfuClient>,
    /// Channel for SFU ICE candidates
    ice_candidate_rx: Option<mpsc::UnboundedReceiver<crate::media::IceCandidate>>,
    /// Current voice channel ID for SFU
    sfu_channel_id: Option<Uuid>,
}

impl VoiceChannelView {
    pub fn new() -> Self {
        Self {
            video_capture: None,
            video_texture: None,
            vad: None,
            remote_textures: HashMap::new(),
            sfu_client: None,
            ice_candidate_rx: None,
            sfu_channel_id: None,
        }
    }

    /// Start local video capture
    pub fn start_video(&mut self, device_index: Option<u32>) {
        if self.video_capture.is_some() {
            return; // Already capturing
        }

        match GstVideoCapture::new() {
            Ok(mut capture) => {
                if let Err(e) = capture.start(device_index) {
                    tracing::error!("Failed to start video capture: {}", e);
                    return;
                }
                tracing::info!("Voice channel video capture started");
                self.video_capture = Some(capture);
            }
            Err(e) => {
                tracing::error!("Failed to create video capture: {}", e);
            }
        }
    }

    /// Stop local video capture
    pub fn stop_video(&mut self) {
        if let Some(mut capture) = self.video_capture.take() {
            capture.stop();
            tracing::info!("Voice channel video capture stopped");
        }
        self.video_texture = None;
    }

    /// Initialize VAD with audio level monitor
    pub fn init_vad(&mut self, level_monitor: Arc<AtomicU32>, threshold_db: f32) {
        self.vad = Some(VoiceActivityDetector::new(level_monitor, threshold_db));
    }

    /// Check if VAD is initialized
    pub fn has_vad(&self) -> bool {
        self.vad.is_some()
    }

    /// Main render function
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        // Update VAD state
        let local_speaking = if let Some(vad) = &mut self.vad {
            let speaking = vad.update();
            // Update state asynchronously
            let state_clone = state.clone();
            runtime.spawn(async move {
                state_clone.set_local_speaking(speaking).await;
            });
            speaking
        } else {
            false
        };

        // Update local video texture if capturing
        if let Some(capture) = &self.video_capture {
            if let Some(frame) = capture.get_frame() {
                let image = ColorImage::from_rgb(
                    [frame.width as usize, frame.height as usize],
                    &frame.data,
                );

                if let Some(texture) = &mut self.video_texture {
                    texture.set(image, TextureOptions::default());
                } else {
                    self.video_texture = Some(ctx.load_texture(
                        "voice_local_video",
                        image,
                        TextureOptions::default(),
                    ));
                }

                // Send frame to SFU if connected
                if let Some(sfu) = &self.sfu_client {
                    let sfu_clone = sfu.clone();
                    let frame_clone = frame.clone();
                    runtime.spawn(async move {
                        if let Err(e) = sfu_clone.send_frame(&frame_clone).await {
                            tracing::debug!("Failed to send frame to SFU: {}", e);
                        }
                    });
                }
            }
        }

        // Get voice state
        let (channel_name, voice_channel_id, participants, current_user_id, is_video_enabled) =
            runtime.block_on(async {
                let s = state.read().await;
                let voice_channel_id = s.voice_channel_id;
                let channel_name = voice_channel_id
                    .and_then(|id| s.channels.get(&id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "Voice Channel".to_string());
                let participants: Vec<_> = s.voice_participants.values().cloned().collect();
                let current_user_id = s.current_user.as_ref().map(|u| u.id);
                let is_video = s.is_video_enabled;
                (channel_name, voice_channel_id, participants, current_user_id, is_video)
            });

        // Manage video capture based on state
        if is_video_enabled && self.video_capture.is_none() {
            let device_index = runtime.block_on(async { state.read().await.selected_video_device });
            self.start_video(device_index);
        } else if !is_video_enabled && self.video_capture.is_some() {
            self.stop_video();
        }

        // Check if any participant has video enabled (need SFU to receive their video)
        let any_video_in_channel = participants.iter().any(|p| p.is_video_enabled);
        let should_connect_sfu = is_video_enabled || any_video_in_channel;

        // Manage SFU connection for video streaming
        self.update_sfu_connection(state, network, runtime, voice_channel_id, should_connect_sfu);

        // Process pending ICE candidates from SFU client
        self.process_ice_candidates(network, runtime);

        // Handle SFU signaling messages from server
        self.handle_sfu_signaling(state, network, runtime);

        // Update remote video textures
        self.update_remote_textures(ctx, runtime);

        // Request repaint for continuous updates (~30 FPS)
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Dark background
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, theme::BG_PRIMARY);

        ui.vertical(|ui| {
            // Header
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.heading(
                    egui::RichText::new(format!("🔊 {}", channel_name))
                        .color(theme::TEXT_NORMAL)
                        .size(20.0),
                );
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(16.0);

            // Participant grid
            let available_size = ui.available_size();
            self.render_participant_grid(
                ui,
                &participants,
                current_user_id,
                local_speaking,
                is_video_enabled,
                available_size,
            );
        });
    }

    fn render_participant_grid(
        &self,
        ui: &mut egui::Ui,
        participants: &[VoiceParticipant],
        current_user_id: Option<Uuid>,
        local_speaking: bool,
        local_video_enabled: bool,
        available_size: Vec2,
    ) {
        let total_participants = participants.len().max(1);

        // Calculate grid dimensions (Discord-like layout)
        let (cols, rows) = Self::calculate_grid(total_participants);

        // Calculate tile size with padding
        let padding = 16.0;
        let gap = 12.0;
        let tile_width = ((available_size.x - padding * 2.0 - gap * (cols as f32 - 1.0)) / cols as f32)
            .min(400.0)
            .max(200.0);
        let tile_height = ((available_size.y - padding * 2.0 - gap * (rows as f32 - 1.0)) / rows as f32)
            .min(300.0)
            .max(150.0);

        ui.horizontal(|ui| {
            ui.add_space(padding);
            ui.vertical(|ui| {
                let mut count = 0;
                let mut row_ui = ui.horizontal(|_| {});

                for participant in participants {
                    if count > 0 && count % cols == 0 {
                        // End current row, start new one
                        row_ui = ui.horizontal(|_| {});
                    }

                    row_ui = ui.horizontal(|ui| {
                        let is_self = current_user_id == Some(participant.user_id);
                        let is_speaking = if is_self {
                            local_speaking
                        } else {
                            participant.is_speaking
                        };
                        let has_video = if is_self {
                            local_video_enabled
                        } else {
                            participant.is_video_enabled
                        };

                        self.render_participant_tile(
                            ui,
                            participant,
                            is_speaking,
                            has_video,
                            is_self,
                            Vec2::new(tile_width, tile_height),
                        );

                        if count % cols != cols - 1 {
                            ui.add_space(gap);
                        }
                    });

                    count += 1;
                }

                // If no participants, show a placeholder
                if participants.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("No one else is here")
                                .color(theme::TEXT_MUTED)
                                .size(16.0),
                        );
                    });
                }
            });
        });
    }

    fn calculate_grid(count: usize) -> (usize, usize) {
        // Discord-like grid layout
        match count {
            0 | 1 => (1, 1),
            2 => (2, 1),
            3..=4 => (2, 2),
            5..=6 => (3, 2),
            7..=9 => (3, 3),
            10..=12 => (4, 3),
            _ => (4, (count + 3) / 4),
        }
    }

    fn render_participant_tile(
        &self,
        ui: &mut egui::Ui,
        participant: &VoiceParticipant,
        is_speaking: bool,
        has_video: bool,
        is_self: bool,
        size: Vec2,
    ) {
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 12.0, theme::BG_TERTIARY);

        // Speaking border (green glow when speaking)
        let border_color = if is_speaking {
            theme::GREEN
        } else {
            theme::BG_ACCENT
        };
        let border_width = if is_speaking { 3.0 } else { 1.0 };
        painter.rect_stroke(rect, 12.0, egui::Stroke::new(border_width, border_color));

        // Content: video or avatar
        let content_rect = rect.shrink(4.0);

        if has_video && is_self {
            // Show local video
            if let Some(texture) = &self.video_texture {
                // Maintain aspect ratio
                let tex_size = texture.size_vec2();
                let scale = (content_rect.width() / tex_size.x)
                    .min(content_rect.height() / tex_size.y);
                let video_size = tex_size * scale;
                let video_rect = egui::Rect::from_center_size(content_rect.center(), video_size);

                painter.image(
                    texture.id(),
                    video_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                // Video not ready, show avatar
                self.render_avatar(&painter, content_rect, &participant.username);
            }
        } else if has_video && !is_self {
            // Remote video - check if we have a texture for this user
            if let Some(texture) = self.remote_textures.get(&participant.user_id) {
                // Maintain aspect ratio
                let tex_size = texture.size_vec2();
                let scale = (content_rect.width() / tex_size.x)
                    .min(content_rect.height() / tex_size.y);
                let video_size = tex_size * scale;
                let video_rect = egui::Rect::from_center_size(content_rect.center(), video_size);

                painter.image(
                    texture.id(),
                    video_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                // No remote video yet, show avatar with camera indicator
                self.render_avatar(&painter, content_rect, &participant.username);
                // Show camera icon overlay to indicate video is enabled
                let icon_pos = egui::pos2(content_rect.right() - 20.0, content_rect.top() + 20.0);
                painter.text(
                    icon_pos,
                    egui::Align2::CENTER_CENTER,
                    "📹",
                    egui::FontId::proportional(16.0),
                    theme::TEXT_NORMAL,
                );
            }
        } else {
            // No video - show avatar
            self.render_avatar(&painter, content_rect, &participant.username);
        }

        // Username overlay at bottom
        let overlay_height = 36.0;
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.max.y - overlay_height),
            rect.max,
        );
        painter.rect_filled(
            name_rect,
            egui::Rounding {
                nw: 0.0,
                ne: 0.0,
                sw: 12.0,
                se: 12.0,
            },
            Color32::from_black_alpha(200),
        );

        // Build name with status icons
        let mut display_name = participant.username.clone();
        if participant.is_muted {
            display_name.push_str(" 🔇");
        }
        if participant.is_deafened {
            display_name.push_str(" 🔇");
        }
        if participant.is_screen_sharing {
            display_name.push_str(" 🖥️");
        }

        // Name text with speaking color
        let name_color = if is_speaking {
            theme::TEXT_NORMAL
        } else {
            theme::TEXT_MUTED
        };

        painter.text(
            name_rect.center(),
            egui::Align2::CENTER_CENTER,
            &display_name,
            egui::FontId::proportional(14.0),
            name_color,
        );
    }

    fn render_avatar(&self, painter: &egui::Painter, rect: egui::Rect, username: &str) {
        let center = rect.center();
        let avatar_radius = rect.width().min(rect.height()) * 0.25;

        // Avatar circle background
        painter.circle_filled(center, avatar_radius, theme::BG_ACCENT);

        // Initial letter
        let initial = username
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            &initial,
            egui::FontId::proportional(avatar_radius * 0.9),
            theme::TEXT_NORMAL,
        );
    }

    /// Update SFU connection based on voice and video state
    fn update_sfu_connection(
        &mut self,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
        voice_channel_id: Option<Uuid>,
        should_connect_sfu: bool,
    ) {
        // should_connect_sfu is already computed by caller (includes any_video_in_channel check)
        let should_connect = voice_channel_id.is_some() && should_connect_sfu;
        let channel_changed = self.sfu_channel_id != voice_channel_id;
        let has_sfu = self.sfu_client.is_some();

        tracing::info!(
            "SFU state check: voice_channel={:?}, should_connect_sfu={}, should_connect={}, channel_changed={}, has_sfu={}",
            voice_channel_id, should_connect_sfu, should_connect, channel_changed, has_sfu
        );

        if should_connect && (self.sfu_client.is_none() || channel_changed) {
            // Connect to SFU
            if let Some(channel_id) = voice_channel_id {
                tracing::info!("Connecting to SFU for channel {}", channel_id);

                // Create ICE candidate channel
                let (ice_tx, ice_rx) = mpsc::unbounded_channel();
                self.ice_candidate_rx = Some(ice_rx);

                // Get ICE servers and connect
                let network_clone = network.clone();
                let state_clone = state.clone();
                let mut sfu = SfuClient::new();

                let result = runtime.block_on(async {
                    // Get ICE servers from server
                    let ice_servers = match network_clone.get_ice_servers().await {
                        Ok(servers) => servers
                            .into_iter()
                            .map(|s| (s.urls[0].clone(), s.username, s.credential))
                            .collect(),
                        Err(e) => {
                            tracing::warn!("Failed to get ICE servers: {}, using defaults", e);
                            vec![("stun:stun.l.google.com:19302".to_string(), None, None)]
                        }
                    };

                    // Connect to SFU
                    match sfu.connect(channel_id, ice_servers, ice_tx).await {
                        Ok(offer_sdp) => {
                            // Send offer to server via WebSocket
                            network_clone.send_sfu_offer(channel_id, offer_sdp).await;
                            Ok(sfu)
                        }
                        Err(e) => Err(e),
                    }
                });

                match result {
                    Ok(sfu) => {
                        self.sfu_client = Some(sfu);
                        self.sfu_channel_id = Some(channel_id);
                        tracing::info!("SFU client connected, offer sent");
                    }
                    Err(e) => {
                        tracing::error!("Failed to connect to SFU: {}", e);
                    }
                }
            }
        } else if !should_connect && self.sfu_client.is_some() {
            // Disconnect from SFU
            tracing::info!("Disconnecting from SFU");
            if let Some(mut sfu) = self.sfu_client.take() {
                runtime.block_on(async {
                    if let Err(e) = sfu.disconnect().await {
                        tracing::warn!("Error disconnecting from SFU: {}", e);
                    }
                });
            }
            self.sfu_channel_id = None;
            self.ice_candidate_rx = None;
            self.remote_textures.clear();
        }
    }

    /// Process pending ICE candidates from the SFU client
    fn process_ice_candidates(&mut self, network: &NetworkClient, runtime: &tokio::runtime::Runtime) {
        if let Some(rx) = &mut self.ice_candidate_rx {
            // Drain all pending ICE candidates
            while let Ok(candidate) = rx.try_recv() {
                let network = network.clone();
                runtime.spawn(async move {
                    network
                        .send_sfu_ice_candidate(
                            candidate.candidate,
                            candidate.sdp_mid,
                            candidate.sdp_mline_index,
                        )
                        .await;
                });
            }
        }
    }

    /// Handle SFU signaling messages from the server
    fn handle_sfu_signaling(
        &mut self,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        if let Some(sfu) = &self.sfu_client {
            // Handle SFU answer
            let answer = runtime.block_on(state.take_sfu_answer());
            if let Some(sdp) = answer {
                let sfu_clone = sfu.clone();
                runtime.block_on(async {
                    if let Err(e) = sfu_clone.handle_answer(sdp).await {
                        tracing::error!("Failed to handle SFU answer: {}", e);
                    }
                });
            }

            // Handle SFU renegotiation
            let renegotiate = runtime.block_on(state.take_sfu_renegotiate());
            if let Some(sdp) = renegotiate {
                let sfu_clone = sfu.clone();
                let network_clone = network.clone();
                let state_clone = state.clone();
                let sdp_clone = sdp.clone();
                runtime.block_on(async {
                    match sfu_clone.handle_renegotiate(sdp).await {
                        Ok(Some(answer_sdp)) => {
                            network_clone.send_sfu_answer(answer_sdp).await;
                        }
                        Ok(None) => {
                            // Not ready yet (waiting for stable state), re-queue for next frame
                            state_clone.set_sfu_renegotiate(sdp_clone).await;
                        }
                        Err(e) => {
                            tracing::error!("Failed to handle SFU renegotiation: {}", e);
                        }
                    }
                });
            }

            // Handle ICE candidates from server
            let candidates = runtime.block_on(state.take_sfu_ice_candidates());
            for candidate in candidates {
                let sfu_clone = sfu.clone();
                runtime.block_on(async {
                    if let Err(e) = sfu_clone
                        .add_ice_candidate(
                            candidate.candidate,
                            candidate.sdp_mid,
                            candidate.sdp_mline_index,
                        )
                        .await
                    {
                        tracing::warn!("Failed to add ICE candidate: {}", e);
                    }
                });
            }
        }
    }

    /// Update remote video textures from SFU
    fn update_remote_textures(&mut self, ctx: &egui::Context, runtime: &tokio::runtime::Runtime) {
        if let Some(sfu) = &self.sfu_client {
            let remote_users = runtime.block_on(sfu.get_remote_users());

            for user_id in remote_users {
                if let Some(frame) = runtime.block_on(sfu.get_remote_frame(user_id)) {
                    if frame.data.len() >= (frame.width * frame.height * 3) as usize {
                        let image = ColorImage::from_rgb(
                            [frame.width as usize, frame.height as usize],
                            &frame.data,
                        );

                        if let Some(texture) = self.remote_textures.get_mut(&user_id) {
                            texture.set(image, TextureOptions::default());
                        } else {
                            let texture = ctx.load_texture(
                                format!("remote_video_{}", user_id),
                                image,
                                TextureOptions::default(),
                            );
                            self.remote_textures.insert(user_id, texture);
                        }
                    }
                }
            }
        }
    }

    /// Clean up resources
    pub fn cleanup(&mut self) {
        self.stop_video();
        self.vad = None;

        // Disconnect SFU
        if let Some(mut sfu) = self.sfu_client.take() {
            // Can't call async disconnect in sync cleanup, just drop it
            drop(sfu);
        }
        self.sfu_channel_id = None;
        self.ice_candidate_rx = None;
        self.remote_textures.clear();
    }
}

impl Default for VoiceChannelView {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VoiceChannelView {
    fn drop(&mut self) {
        self.cleanup();
    }
}
