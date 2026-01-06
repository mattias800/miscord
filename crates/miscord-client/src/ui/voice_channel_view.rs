//! Voice Channel View component
//!
//! Displays a Discord-like grid of participants when in a voice channel.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions, Vec2};
use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use uuid::Uuid;

use crate::media::gst_video::GstVideoCapture;
use crate::media::VoiceActivityDetector;
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
    /// Cached participant textures (for future remote video)
    _participant_textures: HashMap<Uuid, TextureHandle>,
}

impl VoiceChannelView {
    pub fn new() -> Self {
        Self {
            video_capture: None,
            video_texture: None,
            vad: None,
            _participant_textures: HashMap::new(),
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
            }
        }

        // Get voice state
        let (channel_name, participants, current_user_id, is_video_enabled) =
            runtime.block_on(async {
                let s = state.read().await;
                let channel_name = s
                    .voice_channel_id
                    .and_then(|id| s.channels.get(&id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "Voice Channel".to_string());
                let participants: Vec<_> = s.voice_participants.values().cloned().collect();
                let current_user_id = s.current_user.as_ref().map(|u| u.id);
                let is_video = s.is_video_enabled;
                (channel_name, participants, current_user_id, is_video)
            });

        // Manage video capture based on state
        if is_video_enabled && self.video_capture.is_none() {
            let device_index = runtime.block_on(async { state.read().await.selected_video_device });
            self.start_video(device_index);
        } else if !is_video_enabled && self.video_capture.is_some() {
            self.stop_video();
        }

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
            // Remote video - not implemented yet, show avatar with camera indicator
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

    /// Clean up resources
    pub fn cleanup(&mut self) {
        self.stop_video();
        self.vad = None;
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
