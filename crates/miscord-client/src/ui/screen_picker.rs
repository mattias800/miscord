//! Screen Picker Dialog for selecting screen share source
//!
//! Allows users to select a monitor or window to share.

use egui::{Color32, RichText, Vec2};

use crate::media::screen::{MonitorInfo, WindowInfo, ScreenCapture};

/// What type of source is being browsed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenPickerTab {
    Monitors,
    Windows,
}

/// Available resolution presets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    P720,   // 1280x720
    P1080,  // 1920x1080
    P1440,  // 2560x1440
    P2160,  // 3840x2160 (4K)
}

impl Resolution {
    pub fn width(&self) -> u32 {
        match self {
            Resolution::P720 => 1280,
            Resolution::P1080 => 1920,
            Resolution::P1440 => 2560,
            Resolution::P2160 => 3840,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Resolution::P720 => 720,
            Resolution::P1080 => 1080,
            Resolution::P1440 => 1440,
            Resolution::P2160 => 2160,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Resolution::P720 => "720p",
            Resolution::P1080 => "1080p",
            Resolution::P1440 => "1440p",
            Resolution::P2160 => "4K (2160p)",
        }
    }

    pub fn all() -> &'static [Resolution] {
        &[Resolution::P720, Resolution::P1080, Resolution::P1440, Resolution::P2160]
    }
}

/// Available framerate presets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framerate {
    Fps15,
    Fps30,
    Fps60,
}

impl Framerate {
    pub fn value(&self) -> u32 {
        match self {
            Framerate::Fps15 => 15,
            Framerate::Fps30 => 30,
            Framerate::Fps60 => 60,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Framerate::Fps15 => "15 FPS",
            Framerate::Fps30 => "30 FPS",
            Framerate::Fps60 => "60 FPS",
        }
    }

    pub fn all() -> &'static [Framerate] {
        &[Framerate::Fps15, Framerate::Fps30, Framerate::Fps60]
    }
}

/// Selected capture source with settings
#[derive(Debug, Clone)]
pub struct CaptureSource {
    pub source_type: CaptureSourceType,
    pub resolution: Resolution,
    pub framerate: Framerate,
}

/// Type of capture source
#[derive(Debug, Clone)]
pub enum CaptureSourceType {
    Monitor(u32),
    Window(u64),
}

/// Dialog for selecting screen share source
pub struct ScreenPickerDialog {
    /// Available monitors
    monitors: Vec<MonitorInfo>,
    /// Available windows
    windows: Vec<WindowInfo>,
    /// Currently selected tab
    selected_tab: ScreenPickerTab,
    /// Currently selected monitor index (in monitors vec)
    selected_monitor_index: Option<usize>,
    /// Currently selected window index (in windows vec)
    selected_window_index: Option<usize>,
    /// Selected resolution
    selected_resolution: Resolution,
    /// Selected framerate
    selected_framerate: Framerate,
    /// Whether the dialog is open
    is_open: bool,
}

impl ScreenPickerDialog {
    /// Create a new screen picker dialog
    pub fn new() -> Self {
        Self {
            monitors: Vec::new(),
            windows: Vec::new(),
            selected_tab: ScreenPickerTab::Monitors,
            selected_monitor_index: None,
            selected_window_index: None,
            selected_resolution: Resolution::P1080, // Default to 1080p
            selected_framerate: Framerate::Fps30,   // Default to 30 FPS
            is_open: false,
        }
    }

    /// Open the dialog and refresh the list of available sources
    pub fn open(&mut self) {
        // Refresh monitors list
        match ScreenCapture::list_monitors() {
            Ok(monitors) => self.monitors = monitors,
            Err(e) => {
                tracing::warn!("Failed to list monitors: {}", e);
                self.monitors = Vec::new();
            }
        }

        // Refresh windows list
        match ScreenCapture::list_windows() {
            Ok(windows) => self.windows = windows,
            Err(e) => {
                tracing::warn!("Failed to list windows: {}", e);
                self.windows = Vec::new();
            }
        }

        // Reset selection
        self.selected_monitor_index = if !self.monitors.is_empty() {
            Some(0) // Select first monitor by default
        } else {
            None
        };
        self.selected_window_index = None;
        self.selected_tab = ScreenPickerTab::Monitors;
        self.is_open = true;
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.is_open = false;
    }

    /// Check if the dialog is open
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the dialog and return the selected source if user clicks Share
    /// Returns Some(source) when user selects and confirms, None otherwise
    pub fn show(&mut self, ctx: &egui::Context) -> Option<CaptureSource> {
        if !self.is_open {
            return None;
        }

        let mut result = None;
        let mut should_close = false;

        egui::Window::new("Select what to share")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(500.0, 480.0))
            .show(ctx, |ui| {
                // Tab bar
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.selected_tab == ScreenPickerTab::Monitors,
                            "Monitors",
                        )
                        .clicked()
                    {
                        self.selected_tab = ScreenPickerTab::Monitors;
                    }
                    if ui
                        .selectable_label(
                            self.selected_tab == ScreenPickerTab::Windows,
                            "Windows",
                        )
                        .clicked()
                    {
                        self.selected_tab = ScreenPickerTab::Windows;
                    }
                });

                ui.separator();

                // Content area (monitors/windows grid)
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        match self.selected_tab {
                            ScreenPickerTab::Monitors => self.render_monitors(ui),
                            ScreenPickerTab::Windows => self.render_windows(ui),
                        }
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // Quality settings section
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Quality Settings").strong());
                });
                ui.add_space(4.0);

                // Resolution and FPS in a horizontal layout
                ui.horizontal(|ui| {
                    // Resolution dropdown
                    ui.label("Resolution:");
                    egui::ComboBox::from_id_salt("resolution_combo")
                        .selected_text(self.selected_resolution.label())
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for res in Resolution::all() {
                                ui.selectable_value(
                                    &mut self.selected_resolution,
                                    *res,
                                    res.label(),
                                );
                            }
                        });

                    ui.add_space(20.0);

                    // Framerate dropdown
                    ui.label("Framerate:");
                    egui::ComboBox::from_id_salt("framerate_combo")
                        .selected_text(self.selected_framerate.label())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for fps in Framerate::all() {
                                ui.selectable_value(
                                    &mut self.selected_framerate,
                                    *fps,
                                    fps.label(),
                                );
                            }
                        });
                });

                // Show estimated bitrate hint
                let bitrate_hint = match (&self.selected_resolution, &self.selected_framerate) {
                    (Resolution::P720, Framerate::Fps15) => "~1 Mbps",
                    (Resolution::P720, Framerate::Fps30) => "~2 Mbps",
                    (Resolution::P720, Framerate::Fps60) => "~3 Mbps",
                    (Resolution::P1080, Framerate::Fps15) => "~2 Mbps",
                    (Resolution::P1080, Framerate::Fps30) => "~4 Mbps",
                    (Resolution::P1080, Framerate::Fps60) => "~6 Mbps",
                    (Resolution::P1440, Framerate::Fps15) => "~4 Mbps",
                    (Resolution::P1440, Framerate::Fps30) => "~6 Mbps",
                    (Resolution::P1440, Framerate::Fps60) => "~10 Mbps",
                    (Resolution::P2160, Framerate::Fps15) => "~6 Mbps",
                    (Resolution::P2160, Framerate::Fps30) => "~10 Mbps",
                    (Resolution::P2160, Framerate::Fps60) => "~15 Mbps",
                };
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Estimated bandwidth: {}", bitrate_hint)).small().weak());

                ui.add_space(8.0);
                ui.separator();

                // Bottom buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_share = match self.selected_tab {
                            ScreenPickerTab::Monitors => self.selected_monitor_index.is_some(),
                            ScreenPickerTab::Windows => self.selected_window_index.is_some(),
                        };

                        if ui
                            .add_enabled(can_share, egui::Button::new("Share"))
                            .clicked()
                        {
                            result = self.get_selected_source();
                            should_close = true;
                        }

                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }
                    });
                });
            });

        if should_close {
            self.close();
        }

        result
    }

    /// Render the monitors grid
    fn render_monitors(&mut self, ui: &mut egui::Ui) {
        if self.monitors.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("No monitors found");
            });
            return;
        }

        // Create a grid of monitor cards
        let card_size = Vec2::new(140.0, 100.0);
        let spacing = 10.0;
        let available_width = ui.available_width();
        let cards_per_row = ((available_width + spacing) / (card_size.x + spacing)) as usize;
        let cards_per_row = cards_per_row.max(1);

        egui::Grid::new("monitors_grid")
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                for (index, monitor) in self.monitors.iter().enumerate() {
                    let is_selected = self.selected_monitor_index == Some(index);

                    let response = self.render_source_card(
                        ui,
                        card_size,
                        &monitor.name,
                        &format!("{}x{}", monitor.width, monitor.height),
                        is_selected,
                        monitor.is_primary,
                    );

                    if response.clicked() {
                        self.selected_monitor_index = Some(index);
                    }

                    // New row after cards_per_row items
                    if (index + 1) % cards_per_row == 0 {
                        ui.end_row();
                    }
                }
            });
    }

    /// Render the windows grid
    fn render_windows(&mut self, ui: &mut egui::Ui) {
        if self.windows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Window capture is not currently supported.\nUse monitor capture instead.");
            });
            return;
        }

        // Create a grid of window cards
        let card_size = Vec2::new(140.0, 100.0);
        let spacing = 10.0;
        let available_width = ui.available_width();
        let cards_per_row = ((available_width + spacing) / (card_size.x + spacing)) as usize;
        let cards_per_row = cards_per_row.max(1);

        egui::Grid::new("windows_grid")
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                for (index, window) in self.windows.iter().enumerate() {
                    let is_selected = self.selected_window_index == Some(index);
                    let display_name = if window.title.len() > 15 {
                        format!("{}...", &window.title[..15])
                    } else {
                        window.title.clone()
                    };

                    let response = self.render_source_card(
                        ui,
                        card_size,
                        &display_name,
                        &window.app_name,
                        is_selected,
                        false,
                    );

                    if response.clicked() {
                        self.selected_window_index = Some(index);
                    }

                    // New row after cards_per_row items
                    if (index + 1) % cards_per_row == 0 {
                        ui.end_row();
                    }
                }
            });
    }

    /// Render a single source card (monitor or window)
    fn render_source_card(
        &self,
        ui: &mut egui::Ui,
        size: Vec2,
        name: &str,
        subtitle: &str,
        is_selected: bool,
        is_primary: bool,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&response);

            // Background
            let bg_color = if is_selected {
                Color32::from_rgb(60, 100, 180)
            } else if response.hovered() {
                Color32::from_rgb(60, 60, 70)
            } else {
                Color32::from_rgb(45, 45, 55)
            };

            ui.painter().rect_filled(rect, 8.0, bg_color);

            // Border
            let stroke_color = if is_selected {
                Color32::from_rgb(100, 150, 255)
            } else {
                visuals.bg_stroke.color
            };
            ui.painter()
                .rect_stroke(rect, 8.0, egui::Stroke::new(2.0, stroke_color));

            // Icon placeholder (monitor or window icon)
            let icon_rect = egui::Rect::from_min_size(
                rect.min + Vec2::new(10.0, 10.0),
                Vec2::new(rect.width() - 20.0, 40.0),
            );
            ui.painter()
                .rect_filled(icon_rect, 4.0, Color32::from_rgb(30, 30, 40));

            // Name
            let name_pos = rect.min + Vec2::new(10.0, 58.0);
            let mut text = RichText::new(name).size(12.0);
            if is_primary {
                text = text.strong();
            }
            ui.painter().text(
                name_pos,
                egui::Align2::LEFT_TOP,
                text.text(),
                egui::FontId::default(),
                Color32::WHITE,
            );

            // Subtitle
            let subtitle_pos = rect.min + Vec2::new(10.0, 75.0);
            ui.painter().text(
                subtitle_pos,
                egui::Align2::LEFT_TOP,
                subtitle,
                egui::FontId::proportional(10.0),
                Color32::GRAY,
            );

            // Primary badge
            if is_primary {
                let badge_pos = rect.max - Vec2::new(10.0, 10.0);
                ui.painter().text(
                    badge_pos,
                    egui::Align2::RIGHT_BOTTOM,
                    "Primary",
                    egui::FontId::proportional(9.0),
                    Color32::from_rgb(100, 200, 100),
                );
            }
        }

        response
    }

    /// Get the currently selected source
    fn get_selected_source(&self) -> Option<CaptureSource> {
        let source_type = match self.selected_tab {
            ScreenPickerTab::Monitors => {
                self.selected_monitor_index.map(|idx| {
                    CaptureSourceType::Monitor(self.monitors[idx].id)
                })
            }
            ScreenPickerTab::Windows => {
                self.selected_window_index.map(|idx| {
                    CaptureSourceType::Window(self.windows[idx].id)
                })
            }
        };

        source_type.map(|st| CaptureSource {
            source_type: st,
            resolution: self.selected_resolution,
            framerate: self.selected_framerate,
        })
    }
}

impl Default for ScreenPickerDialog {
    fn default() -> Self {
        Self::new()
    }
}
