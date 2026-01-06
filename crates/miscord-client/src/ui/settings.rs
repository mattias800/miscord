//! Settings view with Discord-like left navigation
//!
//! Provides settings management with sections for audio, video, etc.

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions, Ui};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::media::audio::{list_input_devices, list_output_devices, AudioCapture, AudioPlayback, linear_to_db};
use crate::media::gst_video::{GstVideoCapture, VideoDeviceInfo};
use crate::state::{AppState, PersistentSettings};

/// The settings view component
pub struct SettingsView {
    current_section: SettingsSection,
    // Audio test state
    audio_capture: Option<AudioCapture>,
    audio_playback: Option<AudioPlayback>,
    audio_rx: Option<mpsc::Receiver<Vec<f32>>>,
    is_testing_audio: bool,
    input_level: Arc<AtomicU32>,
    // Cached device lists
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    // Video test state
    video_capture: Option<GstVideoCapture>,
    is_testing_video: bool,
    video_texture: Option<TextureHandle>,
    video_devices: Vec<VideoDeviceInfo>,
    // Error message
    error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Audio,
    Video,
    // Future sections
    // Appearance,
    // Notifications,
}

impl SettingsView {
    pub fn new() -> Self {
        Self {
            current_section: SettingsSection::Audio,
            audio_capture: None,
            audio_playback: None,
            audio_rx: None,
            is_testing_audio: false,
            input_level: Arc::new(AtomicU32::new(0)),
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            video_capture: None,
            is_testing_video: false,
            video_texture: None,
            video_devices: Vec::new(),
            error_message: None,
        }
    }

    /// Refresh the audio device lists
    fn refresh_audio_devices(&mut self) {
        self.input_devices = list_input_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|d| {
                        if d.is_default {
                            format!("{} (Default)", d.name)
                        } else {
                            d.name
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        self.output_devices = list_output_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|d| {
                        if d.is_default {
                            format!("{} (Default)", d.name)
                        } else {
                            d.name
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    /// Refresh the video device list
    fn refresh_video_devices(&mut self) {
        self.video_devices = GstVideoCapture::list_devices().unwrap_or_default();
    }

    /// Start audio test (capture and optionally playback)
    fn start_audio_test(&mut self, state: &AppState, runtime: &tokio::runtime::Runtime) {
        self.error_message = None;

        // Get selected device and audio settings from state
        let (input_device, output_device, loopback, gain_db, gate_threshold_db, gate_enabled) =
            runtime.block_on(async {
                let s = state.read().await;
                (
                    s.selected_input_device.clone(),
                    s.selected_output_device.clone(),
                    s.loopback_enabled,
                    s.input_gain_db,
                    s.gate_threshold_db,
                    s.gate_enabled,
                )
            });

        // Strip "(Default)" suffix if present for device lookup
        let input_device_name = input_device.as_ref().map(|s| {
            s.trim_end_matches(" (Default)").to_string()
        });
        let output_device_name = output_device.as_ref().map(|s| {
            s.trim_end_matches(" (Default)").to_string()
        });

        // Start capture
        let mut capture = AudioCapture::new();

        // Apply audio settings (in dB)
        capture.set_gain_db(gain_db);
        capture.set_gate_threshold_db(gate_threshold_db);
        capture.set_gate_enabled(gate_enabled);

        match capture.start(input_device_name.as_deref()) {
            Ok(rx) => {
                self.input_level = capture.level_monitor();

                if loopback {
                    // Start playback for loopback
                    let mut playback = AudioPlayback::new();
                    if let Err(e) = playback.start_with_device(output_device_name.as_deref(), rx) {
                        self.error_message = Some(format!("Playback error: {}", e));
                        capture.stop();
                        return;
                    }
                    self.audio_playback = Some(playback);
                    self.audio_rx = None;
                } else {
                    // Just capture, consume samples to keep level meter working
                    self.audio_rx = Some(rx);
                    self.audio_playback = None;
                }

                self.audio_capture = Some(capture);
                self.is_testing_audio = true;
            }
            Err(e) => {
                self.error_message = Some(format!("Capture error: {}", e));
            }
        }
    }

    /// Stop audio test
    fn stop_audio_test(&mut self) {
        if let Some(mut capture) = self.audio_capture.take() {
            capture.stop();
        }
        if let Some(mut playback) = self.audio_playback.take() {
            playback.stop();
        }
        self.audio_rx = None;
        self.is_testing_audio = false;
        self.input_level = Arc::new(AtomicU32::new(0));
    }

    /// Start video test
    fn start_video_test(&mut self, state: &AppState, runtime: &tokio::runtime::Runtime) {
        self.error_message = None;

        // Get selected device from state
        let device_index = runtime.block_on(async {
            state.read().await.selected_video_device
        });

        match GstVideoCapture::new() {
            Ok(mut capture) => {
                match capture.start(device_index) {
                    Ok(()) => {
                        self.video_capture = Some(capture);
                        self.is_testing_video = true;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Video capture error: {}", e));
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to initialize GStreamer: {}", e));
            }
        }
    }

    /// Stop video test
    fn stop_video_test(&mut self) {
        if let Some(mut capture) = self.video_capture.take() {
            capture.stop();
        }
        self.is_testing_video = false;
        self.video_texture = None;
    }

    /// Save current settings to disk
    fn save_settings(&self, state: &AppState, runtime: &tokio::runtime::Runtime) {
        let settings = runtime.block_on(async {
            let s = state.read().await;
            PersistentSettings {
                input_device: s.selected_input_device.clone(),
                output_device: s.selected_output_device.clone(),
                video_device: s.selected_video_device,
                input_gain_db: Some(s.input_gain_db),
                gate_threshold_db: Some(s.gate_threshold_db),
                gate_enabled: Some(s.gate_enabled),
                loopback_enabled: Some(s.loopback_enabled),
            }
        });
        settings.save();
    }

    /// Render the settings view
    /// Returns true if the close button was pressed
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) -> bool {
        let mut close_requested = false;

        // Refresh devices on first frame or periodically
        if self.input_devices.is_empty() {
            self.refresh_audio_devices();
        }
        if self.video_devices.is_empty() {
            self.refresh_video_devices();
        }

        // Drain audio samples if not in loopback mode (to keep level meter working)
        if let Some(rx) = &mut self.audio_rx {
            while rx.try_recv().is_ok() {}
        }

        // Capture video frame if testing
        if self.is_testing_video {
            if let Some(capture) = &self.video_capture {
                if let Some(frame) = capture.get_frame() {
                    tracing::debug!("Settings: got frame {}x{}", frame.width, frame.height);
                    // Convert RGB to ColorImage
                    let image = ColorImage::from_rgb(
                        [frame.width as usize, frame.height as usize],
                        &frame.data,
                    );

                    // Update texture
                    if let Some(texture) = &mut self.video_texture {
                        texture.set(image, TextureOptions::default());
                    } else {
                        self.video_texture = Some(ctx.load_texture(
                            "video_preview",
                            image,
                            TextureOptions::default(),
                        ));
                    }
                }
            }
            // Request repaint for continuous video updates at ~30 FPS
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }

        // Full screen settings panel
        egui::CentralPanel::default().show(ctx, |ui| {
            // Header with close button
            ui.horizontal(|ui| {
                ui.heading("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("X").clicked() {
                        close_requested = true;
                    }
                });
            });

            ui.separator();

            // Main content with left nav and right content
            ui.horizontal(|ui| {
                // Left navigation panel
                ui.vertical(|ui| {
                    ui.set_min_width(150.0);
                    ui.set_max_width(150.0);

                    ui.add_space(8.0);

                    // Audio section
                    let audio_selected = self.current_section == SettingsSection::Audio;
                    let audio_text = if audio_selected {
                        RichText::new("Audio").strong()
                    } else {
                        RichText::new("Audio")
                    };

                    if ui
                        .selectable_label(audio_selected, audio_text)
                        .clicked()
                    {
                        self.current_section = SettingsSection::Audio;
                    }

                    // Video section
                    let video_selected = self.current_section == SettingsSection::Video;
                    let video_text = if video_selected {
                        RichText::new("Video").strong()
                    } else {
                        RichText::new("Video")
                    };

                    if ui
                        .selectable_label(video_selected, video_text)
                        .clicked()
                    {
                        self.current_section = SettingsSection::Video;
                    }

                    // Future sections can be added here
                    // ui.selectable_label(false, "Appearance");
                });

                ui.separator();

                // Right content panel
                ui.vertical(|ui| {
                    ui.set_min_width(400.0);

                    match self.current_section {
                        SettingsSection::Audio => {
                            self.show_audio_settings(ui, state, runtime);
                        }
                        SettingsSection::Video => {
                            self.show_video_settings(ui, state, runtime);
                        }
                    }
                });
            });
        });

        // Stop tests and save settings if closing
        if close_requested {
            self.stop_audio_test();
            self.stop_video_test();
            self.save_settings(state, runtime);
        }

        close_requested
    }

    /// Render audio settings section
    fn show_audio_settings(
        &mut self,
        ui: &mut Ui,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) {
        ui.heading("Audio Settings");
        ui.add_space(16.0);

        // Track if we need to restart the test
        let mut restart_test = false;

        // Get current state
        let (selected_input, selected_output, loopback, input_gain_db, gate_threshold_db, gate_enabled) =
            runtime.block_on(async {
                let s = state.read().await;
                (
                    s.selected_input_device.clone(),
                    s.selected_output_device.clone(),
                    s.loopback_enabled,
                    s.input_gain_db,
                    s.gate_threshold_db,
                    s.gate_enabled,
                )
            });

        // Clone device lists to avoid borrow conflicts
        let input_devices = self.input_devices.clone();
        let output_devices = self.output_devices.clone();
        let is_testing = self.is_testing_audio;

        // Input Device
        ui.label(RichText::new("Input Device").strong());
        ui.add_space(4.0);

        let current_input = selected_input
            .clone()
            .unwrap_or_else(|| "Default".to_string());

        egui::ComboBox::from_id_salt("input_device")
            .selected_text(&current_input)
            .width(300.0)
            .show_ui(ui, |ui| {
                // Default option
                if ui
                    .selectable_label(selected_input.is_none(), "Default")
                    .clicked()
                {
                    let state = state.clone();
                    runtime.block_on(async {
                        state.write().await.selected_input_device = None;
                    });
                    if is_testing {
                        restart_test = true;
                    }
                }

                // Device options
                for device in &input_devices {
                    let is_selected = selected_input.as_ref() == Some(device);
                    if ui.selectable_label(is_selected, device).clicked() {
                        let state = state.clone();
                        let device = device.clone();
                        runtime.block_on(async {
                            state.write().await.selected_input_device = Some(device);
                        });
                        if is_testing {
                            restart_test = true;
                        }
                    }
                }
            });

        ui.add_space(8.0);

        // Input Gain Slider (dB)
        ui.label(RichText::new("Input Gain").strong());
        ui.add_space(4.0);

        let mut current_gain_db = input_gain_db;
        let gain_response = ui.add(
            egui::Slider::new(&mut current_gain_db, -20.0..=20.0)
                .text("dB")
                .clamp_to_range(true),
        );
        if gain_response.changed() {
            let state = state.clone();
            runtime.block_on(async {
                state.write().await.input_gain_db = current_gain_db;
            });
            // Update capture in real-time if testing
            if let Some(capture) = &self.audio_capture {
                capture.set_gain_db(current_gain_db);
            }
        }

        ui.add_space(8.0);

        // Input Level Meter with dB scale
        ui.label("Input Level");

        // Get level in dB (already stored as dB in the atomic)
        let level_db = f32::from_bits(self.input_level.load(Ordering::Relaxed));

        // Convert dB to position on meter (-60 to 0 dB range)
        let db_to_pos = |db: f32| -> f32 {
            ((db + 60.0) / 60.0).clamp(0.0, 1.0)
        };

        let level_pos = db_to_pos(level_db);
        let gate_pos = db_to_pos(gate_threshold_db);

        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(300.0, 20.0),
            egui::Sense::hover(),
        );

        // Background
        ui.painter().rect_filled(
            rect,
            4.0,
            Color32::from_rgb(40, 42, 54),
        );

        // Level bar
        if level_pos > 0.0 {
            let bar_width = rect.width() * level_pos;
            let bar_rect = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(bar_width, rect.height()),
            );

            // Color based on level - gray if below gate, colored if above
            let is_gated = gate_enabled && level_db < gate_threshold_db;
            let color = if is_gated {
                Color32::from_rgb(100, 100, 100) // Gray when gated
            } else if level_db < -12.0 {
                Color32::from_rgb(67, 181, 129) // Green (below -12 dB)
            } else if level_db < -3.0 {
                Color32::from_rgb(250, 166, 26) // Yellow (-12 to -3 dB)
            } else {
                Color32::from_rgb(240, 71, 71) // Red (above -3 dB)
            };

            ui.painter().rect_filled(bar_rect, 4.0, color);
        }

        // Draw gate threshold line if enabled
        if gate_enabled {
            let gate_x = rect.min.x + rect.width() * gate_pos;
            ui.painter().line_segment(
                [
                    egui::pos2(gate_x, rect.min.y),
                    egui::pos2(gate_x, rect.max.y),
                ],
                egui::Stroke::new(2.0, Color32::from_rgb(255, 255, 255)),
            );
        }

        // Draw dB scale markers
        ui.add_space(2.0);
        let (scale_rect, _) = ui.allocate_exact_size(
            egui::vec2(300.0, 12.0),
            egui::Sense::hover(),
        );

        let db_markers = [-60, -48, -36, -24, -12, -6, 0];
        for &db in &db_markers {
            let pos = db_to_pos(db as f32);
            let x = scale_rect.min.x + scale_rect.width() * pos;

            // Draw tick mark
            ui.painter().line_segment(
                [
                    egui::pos2(x, scale_rect.min.y),
                    egui::pos2(x, scale_rect.min.y + 4.0),
                ],
                egui::Stroke::new(1.0, Color32::from_rgb(150, 150, 150)),
            );

            // Draw label
            let label = format!("{}", db);
            ui.painter().text(
                egui::pos2(x, scale_rect.min.y + 6.0),
                egui::Align2::CENTER_TOP,
                &label,
                egui::FontId::proportional(9.0),
                Color32::from_rgb(150, 150, 150),
            );
        }

        // Show current level in dB
        ui.add_space(4.0);
        let level_text = if level_db <= -60.0 {
            "-∞ dB".to_string()
        } else {
            format!("{:.1} dB", level_db)
        };
        ui.label(RichText::new(level_text).small().weak());

        ui.add_space(8.0);

        // Gate controls
        ui.horizontal(|ui| {
            let mut gate_on = gate_enabled;
            if ui.checkbox(&mut gate_on, "Noise Gate").changed() {
                let state = state.clone();
                runtime.block_on(async {
                    state.write().await.gate_enabled = gate_on;
                });
                if let Some(capture) = &self.audio_capture {
                    capture.set_gate_enabled(gate_on);
                }
            }
        });

        if gate_enabled {
            ui.add_space(4.0);
            let mut current_threshold_db = gate_threshold_db;
            let threshold_response = ui.add(
                egui::Slider::new(&mut current_threshold_db, -60.0..=0.0)
                    .text("dB")
                    .clamp_to_range(true),
            );
            if threshold_response.changed() {
                let state = state.clone();
                runtime.block_on(async {
                    state.write().await.gate_threshold_db = current_threshold_db;
                });
                if let Some(capture) = &self.audio_capture {
                    capture.set_gate_threshold_db(current_threshold_db);
                }
            }
            ui.label(
                RichText::new("Audio below this level will be muted")
                    .small()
                    .weak(),
            );
        }

        ui.add_space(16.0);

        // Output Device
        ui.label(RichText::new("Output Device").strong());
        ui.add_space(4.0);

        let current_output = selected_output
            .clone()
            .unwrap_or_else(|| "Default".to_string());

        egui::ComboBox::from_id_salt("output_device")
            .selected_text(&current_output)
            .width(300.0)
            .show_ui(ui, |ui| {
                // Default option
                if ui
                    .selectable_label(selected_output.is_none(), "Default")
                    .clicked()
                {
                    let state = state.clone();
                    runtime.block_on(async {
                        state.write().await.selected_output_device = None;
                    });
                    // Restart test if active and loopback is on
                    if is_testing && loopback {
                        restart_test = true;
                    }
                }

                // Device options
                for device in &output_devices {
                    let is_selected = selected_output.as_ref() == Some(device);
                    if ui.selectable_label(is_selected, device).clicked() {
                        let state = state.clone();
                        let device = device.clone();
                        runtime.block_on(async {
                            state.write().await.selected_output_device = Some(device);
                        });
                        // Restart test if active and loopback is on
                        if is_testing && loopback {
                            restart_test = true;
                        }
                    }
                }
            });

        ui.add_space(16.0);

        // Test Button
        ui.horizontal(|ui| {
            let button_text = if self.is_testing_audio {
                "Stop Test"
            } else {
                "Test Audio"
            };

            if ui.button(button_text).clicked() {
                if self.is_testing_audio {
                    self.stop_audio_test();
                } else {
                    self.start_audio_test(state, runtime);
                }
            }

            if ui.button("Refresh Devices").clicked() {
                self.refresh_audio_devices();
            }
        });

        ui.add_space(8.0);

        // Loopback toggle
        let mut loopback_enabled = loopback;
        if ui
            .checkbox(&mut loopback_enabled, "Enable Loopback (hear your microphone)")
            .changed()
        {
            let state = state.clone();
            runtime.block_on(async {
                state.write().await.loopback_enabled = loopback_enabled;
            });
            // Restart test if active
            if self.is_testing_audio {
                restart_test = true;
            }
        }

        // Restart test if needed (after all UI code to avoid borrow conflicts)
        if restart_test {
            self.stop_audio_test();
            self.start_audio_test(state, runtime);
        }

        // Error message
        if let Some(error) = &self.error_message {
            ui.add_space(16.0);
            ui.colored_label(Color32::from_rgb(240, 71, 71), error);
        }

        // Help text
        ui.add_space(16.0);
        ui.label(
            RichText::new("Click 'Test Audio' to start capturing audio. Enable 'Loopback' to hear your microphone.")
                .weak()
                .small(),
        );
    }

    /// Render video settings section
    fn show_video_settings(
        &mut self,
        ui: &mut Ui,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) {
        ui.heading("Video Settings");
        ui.add_space(16.0);

        // Get current state
        let selected_device = runtime.block_on(async {
            state.read().await.selected_video_device
        });

        // Clone device list to avoid borrow conflicts
        let video_devices = self.video_devices.clone();

        // Video Device
        ui.label(RichText::new("Camera").strong());
        ui.add_space(4.0);

        let current_device_name = selected_device
            .and_then(|idx| {
                video_devices.iter().find(|d| d.index == idx).map(|d| d.name.clone())
            })
            .unwrap_or_else(|| "Default".to_string());

        egui::ComboBox::from_id_salt("video_device")
            .selected_text(&current_device_name)
            .width(300.0)
            .show_ui(ui, |ui| {
                // Default option (first device or index 0)
                if ui
                    .selectable_label(selected_device.is_none(), "Default")
                    .clicked()
                {
                    let state = state.clone();
                    runtime.block_on(async {
                        state.write().await.selected_video_device = None;
                    });
                    // Restart test if active
                    if self.is_testing_video {
                        self.stop_video_test();
                        self.start_video_test(&state, runtime);
                    }
                }

                // Device options
                for device in &video_devices {
                    let is_selected = selected_device == Some(device.index);
                    if ui.selectable_label(is_selected, &device.name).clicked() {
                        let state = state.clone();
                        let device_index = device.index;
                        runtime.block_on(async {
                            state.write().await.selected_video_device = Some(device_index);
                        });
                        // Restart test if active
                        if self.is_testing_video {
                            self.stop_video_test();
                            self.start_video_test(&state, runtime);
                        }
                    }
                }
            });

        ui.add_space(16.0);

        // Test Button
        ui.horizontal(|ui| {
            let button_text = if self.is_testing_video {
                "Stop Test"
            } else {
                "Test Video"
            };

            if ui.button(button_text).clicked() {
                if self.is_testing_video {
                    self.stop_video_test();
                } else {
                    self.start_video_test(state, runtime);
                }
            }

            if ui.button("Refresh Devices").clicked() {
                self.refresh_video_devices();
            }
        });

        ui.add_space(16.0);

        // Video preview
        if self.is_testing_video {
            ui.label(RichText::new("Preview").strong());
            ui.add_space(8.0);

            if let Some(texture) = &self.video_texture {
                let size = texture.size_vec2();
                // Scale to fit in preview area (max 320x240)
                let max_width = 320.0;
                let max_height = 240.0;
                let scale = (max_width / size.x).min(max_height / size.y).min(1.0);
                let display_size = egui::vec2(size.x * scale, size.y * scale);

                ui.image((texture.id(), display_size));
            } else {
                ui.label("Starting camera...");
                ui.spinner();
            }
        }

        // Error message
        if let Some(error) = &self.error_message {
            ui.add_space(16.0);
            ui.colored_label(Color32::from_rgb(240, 71, 71), error);
        }

        // Help text
        ui.add_space(16.0);
        ui.label(
            RichText::new("Click 'Test Video' to preview your camera.")
                .weak()
                .small(),
        );
    }
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}
