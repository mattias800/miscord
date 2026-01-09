//! Voice Channel View component
//!
//! Displays a Discord-like grid of participants when in a voice channel.
//! Integrates with SFU (Selective Forwarding Unit) for video streaming.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::media::gst_video::GstVideoCapture;
use crate::media::screen::{ScreenCapture, ScreenFrame};
use crate::media::sfu_client::SfuClient;
use crate::media::VoiceActivityDetector;
use crate::network::NetworkClient;
use crate::state::{AppState, VoiceParticipant};

use super::screen_picker::{CaptureSource, CaptureSourceType, ScreenPickerDialog};
use super::theme;

/// Voice channel view showing participant grid
pub struct VoiceChannelView {
    /// Local video capture
    video_capture: Option<GstVideoCapture>,
    /// Local video texture
    video_texture: Option<TextureHandle>,
    /// Local screen capture
    screen_capture: Option<ScreenCapture>,
    /// Local screen texture (preview)
    screen_texture: Option<TextureHandle>,
    /// Voice Activity Detector
    vad: Option<VoiceActivityDetector>,
    /// Remote webcam textures (user_id -> texture)
    remote_textures: HashMap<Uuid, TextureHandle>,
    /// Remote screen share textures (user_id -> texture)
    remote_screen_textures: HashMap<Uuid, TextureHandle>,
    /// Set of remote screen shares we're watching (subscribed to)
    watching_screens: HashSet<Uuid>,
    /// Pending subscription changes: (user_id, subscribe: true/false)
    pending_screen_subscriptions: Vec<(Uuid, bool)>,
    /// SFU client for video streaming
    sfu_client: Option<SfuClient>,
    /// Channel for SFU ICE candidates
    ice_candidate_rx: Option<mpsc::UnboundedReceiver<crate::media::IceCandidate>>,
    /// Current voice channel ID for SFU
    sfu_channel_id: Option<Uuid>,
    /// Screen picker dialog
    screen_picker: ScreenPickerDialog,
    /// FPS for screen sharing (configurable)
    screen_share_fps: u32,
    /// User ID of the screen share currently in fullscreen mode (None = not fullscreen)
    fullscreen_screen_user: Option<Uuid>,
}

impl VoiceChannelView {
    pub fn new() -> Self {
        Self {
            video_capture: None,
            video_texture: None,
            screen_capture: None,
            screen_texture: None,
            vad: None,
            remote_textures: HashMap::new(),
            remote_screen_textures: HashMap::new(),
            watching_screens: HashSet::new(),
            pending_screen_subscriptions: Vec::new(),
            sfu_client: None,
            ice_candidate_rx: None,
            sfu_channel_id: None,
            screen_picker: ScreenPickerDialog::new(),
            screen_share_fps: 30, // Default to 30fps
            fullscreen_screen_user: None,
        }
    }

    /// Toggle fullscreen mode for a screen share
    fn toggle_fullscreen_screen(&mut self, user_id: Uuid) {
        if self.fullscreen_screen_user == Some(user_id) {
            self.fullscreen_screen_user = None;
        } else {
            self.fullscreen_screen_user = Some(user_id);
        }
    }

    /// Check if a screen share is in fullscreen mode
    fn is_screen_fullscreen(&self, user_id: Uuid) -> bool {
        self.fullscreen_screen_user == Some(user_id)
    }

    /// Open the screen picker dialog
    pub fn open_screen_picker(&mut self) {
        self.screen_picker.open();
    }

    /// Check if screen picker is open
    pub fn is_screen_picker_open(&self) -> bool {
        self.screen_picker.is_open()
    }

    /// Start screen sharing from a selected source with quality settings
    pub fn start_screen_share(&mut self, source: CaptureSource) {
        if self.screen_capture.is_some() {
            tracing::warn!("Screen capture already active");
            return;
        }

        let fps = source.framerate.value();
        let max_width = source.resolution.width();
        let max_height = source.resolution.height();

        tracing::info!(
            "Starting screen share: {}x{} @{} fps",
            max_width, max_height, fps
        );

        match ScreenCapture::new_with_scaling(max_width, max_height) {
            Ok(mut capture) => {
                let result = match &source.source_type {
                    CaptureSourceType::Monitor(monitor_id) => capture.start_monitor(*monitor_id, fps),
                    CaptureSourceType::Window(_window_id) => {
                        Err(anyhow::anyhow!("Window capture not supported yet"))
                    }
                };

                match result {
                    Ok(()) => {
                        tracing::info!("Screen capture started at {}x{} @{} fps", max_width, max_height, fps);
                        self.screen_capture = Some(capture);
                        self.screen_share_fps = fps;
                    }
                    Err(e) => {
                        tracing::error!("Failed to start screen capture: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to create screen capture: {}", e);
            }
        }
    }

    /// Stop screen sharing
    pub fn stop_screen_share(&mut self) {
        if let Some(mut capture) = self.screen_capture.take() {
            capture.stop();
            tracing::info!("Screen capture stopped");
        }
        self.screen_texture = None;
    }

    /// Check if screen sharing is active
    pub fn is_screen_sharing(&self) -> bool {
        self.screen_capture.is_some()
    }

    /// Toggle watching a remote screen share
    /// Queues the subscription change to be processed in show() where network is available
    pub fn toggle_watch_screen(&mut self, user_id: Uuid) {
        if self.watching_screens.contains(&user_id) {
            self.watching_screens.remove(&user_id);
            self.remote_screen_textures.remove(&user_id);
            self.pending_screen_subscriptions.push((user_id, false)); // Unsubscribe
            tracing::info!("Stopped watching screen share from {}", user_id);
        } else {
            self.watching_screens.insert(user_id);
            self.pending_screen_subscriptions.push((user_id, true)); // Subscribe
            tracing::info!("Started watching screen share from {}", user_id);
        }
    }

    /// Check if we're watching a specific user's screen
    pub fn is_watching_screen(&self, user_id: Uuid) -> bool {
        self.watching_screens.contains(&user_id)
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
                let image = ColorImage::from_rgba_unmultiplied(
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
        let (channel_name, voice_channel_id, participants, current_user_id, is_video_enabled, is_screen_sharing_state, wants_screen_share) =
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
                let is_screen = s.is_screen_sharing;
                let wants_screen = s.wants_screen_share;
                (channel_name, voice_channel_id, participants, current_user_id, is_video, is_screen, wants_screen)
            });

        // Manage video capture based on state
        if is_video_enabled && self.video_capture.is_none() {
            let device_index = runtime.block_on(async { state.read().await.selected_video_device });
            self.start_video(device_index);
        } else if !is_video_enabled && self.video_capture.is_some() {
            self.stop_video();
        }

        // Handle screen picker dialog - user wants to start sharing
        if wants_screen_share && !self.screen_picker.is_open() && !self.is_screen_sharing() {
            self.open_screen_picker();
        }

        // Handle screen picker result
        let picker_was_open = self.screen_picker.is_open();
        if let Some(source) = self.screen_picker.show(ctx) {
            // User selected a source - start screen capture with selected settings
            self.start_screen_share(source);

            // Update voice state on server since capture started
            if self.is_screen_sharing() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    if network.update_voice_state(None, None, None, Some(true)).await.is_ok() {
                        let mut s = state.write().await;
                        s.is_screen_sharing = true;
                        s.wants_screen_share = false;
                    }
                });
            }
        }

        // Clear wants_screen_share if picker was closed without selecting (cancelled)
        if picker_was_open && !self.screen_picker.is_open() && !self.is_screen_sharing() {
            let state = state.clone();
            runtime.spawn(async move {
                let mut s = state.write().await;
                s.wants_screen_share = false;
            });
        }

        // Stop screen sharing if state says to stop (but not if we just started - wait for state to sync)
        // Don't stop if wants_screen_share is true (user just clicked share, state hasn't synced yet)
        if !is_screen_sharing_state && !wants_screen_share && self.is_screen_sharing() {
            self.stop_screen_share();
        }

        // Update local screen texture and send to SFU
        if let Some(capture) = &self.screen_capture {
            if let Some(screen_frame) = capture.get_frame() {
                let image = ColorImage::from_rgba_unmultiplied(
                    [screen_frame.width as usize, screen_frame.height as usize],
                    &screen_frame.data,
                );

                if let Some(texture) = &mut self.screen_texture {
                    texture.set(image, TextureOptions::default());
                } else {
                    self.screen_texture = Some(ctx.load_texture(
                        "voice_local_screen",
                        image,
                        TextureOptions::default(),
                    ));
                }

                // Send screen frame to SFU if connected
                if let Some(sfu) = &self.sfu_client {
                    let sfu_clone = sfu.clone();
                    let video_frame = crate::media::gst_video::VideoFrame {
                        width: screen_frame.width,
                        height: screen_frame.height,
                        data: screen_frame.data.clone(),
                    };
                    let fps = self.screen_share_fps;
                    runtime.spawn(async move {
                        if let Err(e) = sfu_clone.send_screen_frame(&video_frame, fps).await {
                            tracing::debug!("Failed to send screen frame to SFU: {}", e);
                        }
                    });
                }
            }
        }

        // Check if any participant has video or screen sharing enabled (need SFU)
        let any_video_in_channel = participants.iter().any(|p| p.is_video_enabled || p.is_screen_sharing);
        let should_connect_sfu = is_video_enabled || is_screen_sharing_state || self.is_screen_sharing() || any_video_in_channel;

        // Manage SFU connection for video streaming
        self.update_sfu_connection(state, network, runtime, voice_channel_id, should_connect_sfu);

        // Start SFU screen track if local capture is active but SFU track isn't
        if self.is_screen_sharing() {
            if let Some(sfu) = &self.sfu_client {
                // Check if SFU screen track needs to be created
                let sfu_clone = sfu.clone();
                let needs_screen_track = runtime.block_on(async {
                    !sfu_clone.is_screen_sharing().await
                });

                if needs_screen_track {
                    let sfu_clone = sfu.clone();
                    let network_clone = network.clone();
                    let state_clone = state.clone();
                    runtime.spawn(async move {
                        // Create SFU screen track
                        if let Err(e) = sfu_clone.start_screen_share().await {
                            tracing::error!("Failed to start SFU screen share: {}", e);
                            return;
                        }
                        tracing::info!("SFU screen share track created");

                        // Trigger renegotiation by creating new offer
                        match sfu_clone.create_offer().await {
                            Ok(offer_sdp) => {
                                let channel_id = {
                                    let s = state_clone.read().await;
                                    s.voice_channel_id
                                };
                                if let Some(channel_id) = channel_id {
                                    network_clone.send_sfu_offer(channel_id, offer_sdp).await;
                                    tracing::info!("Sent renegotiation offer for screen share");
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to create offer for screen share: {}", e);
                            }
                        }
                    });
                }
            }
        }

        // Process pending ICE candidates from SFU client
        self.process_ice_candidates(network, runtime);

        // Handle SFU signaling messages from server
        self.handle_sfu_signaling(state, network, runtime);

        // Process pending screen subscription changes
        self.process_pending_subscriptions(network, runtime);

        // Update remote video textures (webcam and screen)
        self.update_remote_textures(ctx, runtime);

        // Request repaint for continuous updates (~30 FPS)
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Dark background
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, theme::BG_PRIMARY);

        // Check if we're in fullscreen mode for a screen share
        if let Some(fullscreen_user_id) = self.fullscreen_screen_user {
            // Find the participant info for the fullscreen user
            let participant = participants.iter().find(|p| p.user_id == fullscreen_user_id);

            // If user is no longer sharing or not found, exit fullscreen
            if participant.map(|p| p.is_screen_sharing).unwrap_or(false) {
                self.render_fullscreen_screen(ui, fullscreen_user_id, participant.map(|p| p.username.as_str()).unwrap_or("Unknown"));
                return;
            } else {
                // User stopped sharing, exit fullscreen
                self.fullscreen_screen_user = None;
            }
        }

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
                is_screen_sharing_state,
                available_size,
            );
        });
    }

    /// Render fullscreen view for a screen share
    fn render_fullscreen_screen(&mut self, ui: &mut egui::Ui, user_id: Uuid, username: &str) {
        let rect = ui.available_rect_before_wrap();
        let (response_rect, response) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
        let painter = ui.painter_at(response_rect);

        // Black background for fullscreen
        painter.rect_filled(response_rect, 0.0, Color32::BLACK);

        // Render the screen share texture
        if let Some(texture) = self.remote_screen_textures.get(&user_id) {
            let tex_size = texture.size_vec2();
            let scale = (response_rect.width() / tex_size.x)
                .min(response_rect.height() / tex_size.y);
            let video_size = tex_size * scale;
            let video_rect = egui::Rect::from_center_size(response_rect.center(), video_size);

            painter.image(
                texture.id(),
                video_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            // No frames yet
            painter.text(
                response_rect.center(),
                egui::Align2::CENTER_CENTER,
                "🖥️ Loading...",
                egui::FontId::proportional(24.0),
                theme::TEXT_MUTED,
            );
        }

        // Exit fullscreen button in upper right corner
        let button_size = 36.0;
        let button_margin = 16.0;
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(
                response_rect.max.x - button_size - button_margin,
                response_rect.min.y + button_margin,
            ),
            Vec2::splat(button_size),
        );

        // Check if mouse is over the button
        let mouse_pos = ui.input(|i| i.pointer.hover_pos());
        let button_hovered = mouse_pos.map(|p| button_rect.contains(p)).unwrap_or(false);

        // Draw button background
        let button_bg = if button_hovered {
            Color32::from_rgba_unmultiplied(255, 255, 255, 80)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 150)
        };
        painter.rect_filled(button_rect, 6.0, button_bg);

        // Draw exit fullscreen icon
        painter.text(
            button_rect.center(),
            egui::Align2::CENTER_CENTER,
            "✕",
            egui::FontId::proportional(20.0),
            Color32::WHITE,
        );

        // Username label at bottom
        let label_height = 40.0;
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(response_rect.min.x, response_rect.max.y - label_height),
            response_rect.max,
        );
        painter.rect_filled(label_rect, 0.0, Color32::from_black_alpha(180));
        painter.text(
            label_rect.center(),
            egui::Align2::CENTER_CENTER,
            &format!("{}'s screen", username),
            egui::FontId::proportional(16.0),
            Color32::WHITE,
        );

        // Handle click - exit fullscreen when clicking the button
        if response.clicked() && button_hovered {
            self.fullscreen_screen_user = None;
        }
    }

    fn render_participant_grid(
        &mut self,
        ui: &mut egui::Ui,
        participants: &[VoiceParticipant],
        current_user_id: Option<Uuid>,
        local_speaking: bool,
        local_video_enabled: bool,
        local_screen_enabled: bool,
        available_size: Vec2,
    ) {
        // Collect all tiles to render
        let mut tiles: Vec<(VoiceParticipant, bool, bool, bool, bool)> = Vec::new(); // (participant, is_self, is_speaking, has_video, is_screen_tile)

        for participant in participants {
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
            let has_screen = if is_self {
                local_screen_enabled
            } else {
                participant.is_screen_sharing
            };

            // Add webcam/avatar tile
            tiles.push((participant.clone(), is_self, is_speaking, has_video, false));

            // Add screen share tile if sharing
            if has_screen {
                tiles.push((participant.clone(), is_self, is_speaking, has_video, true));
            }
        }

        let total_tiles = tiles.len().max(1);

        // Calculate how many columns can fit based on available width
        let padding = 16.0;
        let gap = 12.0;
        let min_tile_width = 250.0;
        let max_tile_width = 400.0;

        // Calculate max columns that can fit
        let max_cols = ((available_size.x - padding * 2.0 + gap) / (min_tile_width + gap))
            .floor()
            .max(1.0) as usize;

        // Use Discord-like grid calculation but cap at max_cols
        let (mut cols, _rows) = Self::calculate_grid(total_tiles);
        cols = cols.min(max_cols);

        // Calculate tile size
        let tile_width = ((available_size.x - padding * 2.0 - gap * (cols as f32 - 1.0)) / cols as f32)
            .min(max_tile_width)
            .max(min_tile_width);
        let tile_height = (tile_width * 0.75).min(300.0).max(150.0); // 4:3 aspect ratio

        let tile_size = Vec2::new(tile_width, tile_height);

        // Wrap in ScrollArea for scrollbar support
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(padding);

                // If no participants, show a placeholder
                if tiles.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("No one else is here")
                                .color(theme::TEXT_MUTED)
                                .size(16.0),
                        );
                    });
                    return;
                }

                // Render tiles in a proper grid
                let mut current_col = 0;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                    ui.set_min_width(available_size.x - padding * 2.0);

                    for (participant, is_self, is_speaking, has_video, is_screen_tile) in &tiles {
                        if *is_screen_tile {
                            self.render_screen_tile(
                                ui,
                                participant,
                                *is_self,
                                tile_size,
                            );
                        } else {
                            self.render_participant_tile(
                                ui,
                                participant,
                                *is_speaking,
                                *has_video,
                                *is_self,
                                tile_size,
                            );
                        }

                        current_col += 1;
                        if current_col >= cols {
                            current_col = 0;
                            ui.end_row();
                        }
                    }
                });

                ui.add_space(padding);
            });
    }

    /// Render a screen share tile for a participant
    fn render_screen_tile(
        &mut self,
        ui: &mut egui::Ui,
        participant: &VoiceParticipant,
        is_self: bool,
        size: Vec2,
    ) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 12.0, theme::BG_TERTIARY);

        // Border (purple for screen share)
        let border_color = Color32::from_rgb(138, 43, 226); // Purple
        painter.rect_stroke(rect, 12.0, egui::Stroke::new(2.0, border_color));

        let content_rect = rect.shrink(4.0);

        if is_self {
            // Show local screen capture preview
            if let Some(texture) = &self.screen_texture {
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
                // Screen capture starting...
                painter.text(
                    content_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "🖥️ Starting...",
                    egui::FontId::proportional(16.0),
                    theme::TEXT_MUTED,
                );
            }
        } else {
            // Remote screen share
            let is_watching = self.watching_screens.contains(&participant.user_id);

            if is_watching {
                // Show remote screen if we have it
                if let Some(texture) = self.remote_screen_textures.get(&participant.user_id) {
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
                    // Subscribed but no frames yet
                    painter.text(
                        content_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "🖥️ Loading...",
                        egui::FontId::proportional(16.0),
                        theme::TEXT_MUTED,
                    );
                }

                // Fullscreen button in upper right corner
                let is_fullscreen = self.is_screen_fullscreen(participant.user_id);
                let button_size = 28.0;
                let button_margin = 8.0;
                let button_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        rect.max.x - button_size - button_margin,
                        rect.min.y + button_margin,
                    ),
                    Vec2::splat(button_size),
                );

                // Check if mouse is over the button
                let mouse_pos = ui.input(|i| i.pointer.hover_pos());
                let button_hovered = mouse_pos.map(|p| button_rect.contains(p)).unwrap_or(false);

                // Draw button background
                let button_bg = if button_hovered {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 60)
                } else {
                    Color32::from_rgba_unmultiplied(0, 0, 0, 120)
                };
                painter.rect_filled(button_rect, 4.0, button_bg);

                // Draw fullscreen icon (expand or compress)
                let icon = if is_fullscreen { "⛶" } else { "⛶" }; // Use same icon, could use different ones
                painter.text(
                    button_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(16.0),
                    Color32::WHITE,
                );

                // Handle button click (check button first, then main area)
                if response.clicked() {
                    if button_hovered {
                        self.toggle_fullscreen_screen(participant.user_id);
                    } else {
                        // Click outside button stops watching
                        self.toggle_watch_screen(participant.user_id);
                    }
                }
            } else {
                // Not watching - show "Watch" prompt
                painter.text(
                    content_rect.center() - Vec2::new(0.0, 15.0),
                    egui::Align2::CENTER_CENTER,
                    "🖥️ Screen Share",
                    egui::FontId::proportional(14.0),
                    theme::TEXT_MUTED,
                );

                // Draw a "Watch" button area
                let button_rect = egui::Rect::from_center_size(
                    content_rect.center() + Vec2::new(0.0, 20.0),
                    Vec2::new(80.0, 28.0),
                );
                let button_color = if response.hovered() {
                    Color32::from_rgb(88, 101, 242) // Discord blurple
                } else {
                    Color32::from_rgb(71, 82, 196)
                };
                painter.rect_filled(button_rect, 4.0, button_color);
                painter.text(
                    button_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Watch",
                    egui::FontId::proportional(13.0),
                    Color32::WHITE,
                );

                // Click to start watching
                if response.clicked() {
                    self.toggle_watch_screen(participant.user_id);
                }
            }
        }

        // Label at bottom
        let overlay_height = 28.0;
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

        let label = format!("{}'s screen", participant.username);
        painter.text(
            name_rect.center(),
            egui::Align2::CENTER_CENTER,
            &label,
            egui::FontId::proportional(12.0),
            theme::TEXT_MUTED,
        );
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
            self.remote_screen_textures.clear();
            self.watching_screens.clear();
            self.fullscreen_screen_user = None;
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

    /// Process pending screen subscription changes
    fn process_pending_subscriptions(&mut self, network: &NetworkClient, runtime: &tokio::runtime::Runtime) {
        // Drain all pending subscription changes
        let pending: Vec<_> = self.pending_screen_subscriptions.drain(..).collect();
        for (user_id, subscribe) in pending {
            let network = network.clone();
            if subscribe {
                runtime.spawn(async move {
                    network.subscribe_screen_track(user_id).await;
                });
            } else {
                runtime.spawn(async move {
                    network.unsubscribe_screen_track(user_id).await;
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

            // Handle keyframe requests from server (when a new subscriber joins)
            let keyframe_requests = runtime.block_on(state.take_pending_keyframe_requests());
            for track_type in keyframe_requests {
                if let Err(e) = sfu.force_keyframe(track_type) {
                    tracing::warn!("Failed to force keyframe for {:?}: {}", track_type, e);
                }
            }
        }
    }

    /// Update remote video textures from SFU (webcam and screen)
    fn update_remote_textures(&mut self, ctx: &egui::Context, runtime: &tokio::runtime::Runtime) {
        if let Some(sfu) = &self.sfu_client {
            // Update webcam textures
            let remote_users = runtime.block_on(sfu.get_remote_users());

            for user_id in remote_users {
                if let Some(frame) = runtime.block_on(sfu.get_remote_webcam_frame(user_id)) {
                    // Validate frame dimensions to prevent panics from corrupted data
                    let expected_size = (frame.width as usize)
                        .saturating_mul(frame.height as usize)
                        .saturating_mul(4);

                    // Skip invalid frames (corrupted, zero-size, or mismatched data)
                    if frame.width == 0 || frame.height == 0
                        || frame.width > 8192 || frame.height > 8192
                        || frame.data.len() != expected_size
                    {
                        tracing::warn!(
                            "Skipping invalid webcam frame from {}: {}x{}, data_len={}, expected={}",
                            user_id, frame.width, frame.height, frame.data.len(), expected_size
                        );
                        continue;
                    }

                    let image = ColorImage::from_rgba_unmultiplied(
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

            // Update screen share textures for users we're watching
            for &user_id in &self.watching_screens.clone() {
                if let Some(frame) = runtime.block_on(sfu.get_remote_screen_frame(user_id)) {
                    // Validate frame dimensions to prevent panics from corrupted data
                    let expected_size = (frame.width as usize)
                        .saturating_mul(frame.height as usize)
                        .saturating_mul(4);

                    // Skip invalid frames (corrupted, zero-size, or mismatched data)
                    if frame.width == 0 || frame.height == 0
                        || frame.width > 8192 || frame.height > 8192
                        || frame.data.len() != expected_size
                    {
                        tracing::warn!(
                            "Skipping invalid screen frame from {}: {}x{}, data_len={}, expected={}",
                            user_id, frame.width, frame.height, frame.data.len(), expected_size
                        );
                        continue;
                    }

                    // LATENCY MEASUREMENT: Log UI display timestamp
                    static DISPLAY_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let count = DISPLAY_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if count % 30 == 0 {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        tracing::info!("[LATENCY] UI_DISPLAY frame={} ts={} user={}", count, ts, user_id);
                    }

                    let image = ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.data,
                    );

                    if let Some(texture) = self.remote_screen_textures.get_mut(&user_id) {
                        texture.set(image, TextureOptions::default());
                    } else {
                        let texture = ctx.load_texture(
                            format!("remote_screen_{}", user_id),
                            image,
                            TextureOptions::default(),
                        );
                        self.remote_screen_textures.insert(user_id, texture);
                    }
                }
            }
        }
    }

    /// Clean up resources
    pub fn cleanup(&mut self) {
        self.stop_video();
        self.stop_screen_share();
        self.vad = None;

        // Disconnect SFU
        if let Some(mut sfu) = self.sfu_client.take() {
            // Can't call async disconnect in sync cleanup, just drop it
            drop(sfu);
        }
        self.sfu_channel_id = None;
        self.ice_candidate_rx = None;
        self.remote_textures.clear();
        self.remote_screen_textures.clear();
        self.watching_screens.clear();
        self.fullscreen_screen_user = None;
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
