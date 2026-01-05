//! Settings view with Discord-like left navigation
//!
//! Provides settings management with sections for audio, video, etc.

use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::media::audio::{list_input_devices, list_output_devices, AudioCapture, AudioPlayback};
use crate::state::AppState;

/// The settings view component
pub struct SettingsView {
    current_section: SettingsSection,
    // Audio test state
    audio_capture: Option<AudioCapture>,
    audio_playback: Option<AudioPlayback>,
    audio_rx: Option<mpsc::Receiver<Vec<f32>>>,
    is_testing: bool,
    input_level: Arc<AtomicU32>,
    // Cached device lists
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    // Error message
    error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Audio,
    // Future sections
    // Video,
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
            is_testing: false,
            input_level: Arc::new(AtomicU32::new(0)),
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            error_message: None,
        }
    }

    /// Refresh the device lists
    fn refresh_devices(&mut self) {
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

    /// Start audio test (capture and optionally playback)
    fn start_test(&mut self, state: &AppState, runtime: &tokio::runtime::Runtime) {
        self.error_message = None;

        // Get selected device from state
        let (input_device, output_device, loopback) = runtime.block_on(async {
            let s = state.read().await;
            (
                s.selected_input_device.clone(),
                s.selected_output_device.clone(),
                s.loopback_enabled,
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
                self.is_testing = true;
            }
            Err(e) => {
                self.error_message = Some(format!("Capture error: {}", e));
            }
        }
    }

    /// Stop audio test
    fn stop_test(&mut self) {
        if let Some(mut capture) = self.audio_capture.take() {
            capture.stop();
        }
        if let Some(mut playback) = self.audio_playback.take() {
            playback.stop();
        }
        self.audio_rx = None;
        self.is_testing = false;
        self.input_level = Arc::new(AtomicU32::new(0));
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
            self.refresh_devices();
        }

        // Drain audio samples if not in loopback mode (to keep level meter working)
        if let Some(rx) = &mut self.audio_rx {
            while rx.try_recv().is_ok() {}
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

                    // Future sections can be added here
                    // ui.selectable_label(false, "Video");
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
                    }
                });
            });
        });

        // Stop test if closing
        if close_requested {
            self.stop_test();
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
        let (selected_input, selected_output, loopback) = runtime.block_on(async {
            let s = state.read().await;
            (
                s.selected_input_device.clone(),
                s.selected_output_device.clone(),
                s.loopback_enabled,
            )
        });

        // Clone device lists to avoid borrow conflicts
        let input_devices = self.input_devices.clone();
        let output_devices = self.output_devices.clone();
        let is_testing = self.is_testing;

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

        // Input Level Meter
        ui.label("Input Level");
        let level = f32::from_bits(self.input_level.load(Ordering::Relaxed));
        // Scale level for better visibility (audio RMS is usually quite low)
        let scaled_level = (level * 10.0).min(1.0);

        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(300.0, 16.0),
            egui::Sense::hover(),
        );

        // Background
        ui.painter().rect_filled(
            rect,
            4.0,
            Color32::from_rgb(40, 42, 54),
        );

        // Level bar with gradient
        if scaled_level > 0.0 {
            let bar_width = rect.width() * scaled_level;
            let bar_rect = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(bar_width, rect.height()),
            );

            // Color based on level
            let color = if scaled_level < 0.6 {
                Color32::from_rgb(67, 181, 129) // Green
            } else if scaled_level < 0.85 {
                Color32::from_rgb(250, 166, 26) // Yellow
            } else {
                Color32::from_rgb(240, 71, 71) // Red
            };

            ui.painter().rect_filled(bar_rect, 4.0, color);
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
            let button_text = if self.is_testing {
                "Stop Test"
            } else {
                "Test Audio"
            };

            if ui.button(button_text).clicked() {
                if self.is_testing {
                    self.stop_test();
                } else {
                    self.start_test(state, runtime);
                }
            }

            if ui.button("Refresh Devices").clicked() {
                self.refresh_devices();
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
            if self.is_testing {
                restart_test = true;
            }
        }

        // Restart test if needed (after all UI code to avoid borrow conflicts)
        if restart_test {
            self.stop_test();
            self.start_test(state, runtime);
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
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}
