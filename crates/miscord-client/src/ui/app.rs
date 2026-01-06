use eframe::egui;

use crate::network::NetworkClient;
use crate::state::{AppState, PersistentSettings};

use super::login::LoginView;
use super::main_view::MainView;
use super::settings::SettingsView;
use super::theme;

pub struct MiscordApp {
    state: AppState,
    network: NetworkClient,
    runtime: tokio::runtime::Runtime,
    view: View,
    login_view: LoginView,
    main_view: MainView,
    settings_view: SettingsView,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Login,
    Main,
    Settings,
}

impl MiscordApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Set up Discord-like dark theme
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);

        // Apply Discord colors to visuals
        let visuals = &mut style.visuals;
        visuals.dark_mode = true;
        visuals.panel_fill = theme::BG_SECONDARY;
        visuals.window_fill = theme::BG_PRIMARY;
        visuals.extreme_bg_color = theme::BG_TERTIARY;
        visuals.faint_bg_color = theme::BG_ACCENT;

        // Widget styling
        visuals.widgets.noninteractive.bg_fill = theme::BG_PRIMARY;
        visuals.widgets.inactive.bg_fill = theme::BG_PRIMARY;
        visuals.widgets.hovered.bg_fill = theme::BG_ACCENT;
        visuals.widgets.active.bg_fill = theme::BLURPLE;

        // Text colors
        visuals.widgets.noninteractive.fg_stroke.color = theme::TEXT_NORMAL;
        visuals.widgets.inactive.fg_stroke.color = theme::TEXT_MUTED;
        visuals.widgets.hovered.fg_stroke.color = theme::TEXT_NORMAL;
        visuals.widgets.active.fg_stroke.color = theme::TEXT_NORMAL;

        // Selection color
        visuals.selection.bg_fill = theme::BLURPLE;
        visuals.selection.stroke.color = theme::TEXT_NORMAL;

        // Hyperlinks
        visuals.hyperlink_color = theme::TEXT_LINK;

        cc.egui_ctx.set_style(style);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        let state = AppState::new();
        let network = NetworkClient::new(state.clone());

        // Load persistent settings and apply to state
        let persistent_settings = PersistentSettings::load();
        runtime.block_on(async {
            let mut s = state.write().await;
            if let Some(device) = &persistent_settings.input_device {
                s.selected_input_device = Some(device.clone());
            }
            if let Some(device) = &persistent_settings.output_device {
                s.selected_output_device = Some(device.clone());
            }
            if let Some(device) = persistent_settings.video_device {
                s.selected_video_device = Some(device);
            }
            if let Some(gain) = persistent_settings.input_gain_db {
                s.input_gain_db = gain;
            }
            if let Some(threshold) = persistent_settings.gate_threshold_db {
                s.gate_threshold_db = threshold;
            }
            if let Some(enabled) = persistent_settings.gate_enabled {
                s.gate_enabled = enabled;
            }
            if let Some(enabled) = persistent_settings.loopback_enabled {
                s.loopback_enabled = enabled;
            }
        });

        Self {
            state,
            network,
            runtime,
            view: View::Login,
            login_view: LoginView::new(),
            main_view: MainView::new(),
            settings_view: SettingsView::new(),
        }
    }

    /// Open settings view
    pub fn open_settings(&mut self) {
        self.view = View::Settings;
    }

    /// Close settings and return to main view
    pub fn close_settings(&mut self) {
        self.view = View::Main;
    }

    fn check_auth_state(&mut self) {
        let state = self.state.clone();
        let is_auth = self.runtime.block_on(async { state.is_authenticated().await });

        if is_auth {
            self.view = View::Main;
        } else {
            self.view = View::Login;
        }
    }
}

impl eframe::App for MiscordApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request continuous repainting for real-time updates
        ctx.request_repaint();

        match &self.view {
            View::Login => {
                // Try auto-login first if credentials are available
                let login_result = if self.login_view.should_auto_login() {
                    self.login_view
                        .try_auto_login(&self.network, &self.runtime)
                } else {
                    self.login_view.show(ctx, &self.network, &self.runtime)
                };

                if let Some((token, user)) = login_result {
                    let state = self.state.clone();
                    let network = self.network.clone();
                    self.runtime.block_on(async {
                        state.set_auth(token, user).await;
                        // Load communities before switching to Main view
                        if let Err(e) = network.load_communities().await {
                            tracing::error!("Failed to load communities: {}", e);
                        }
                    });
                    self.view = View::Main;

                    // Connect WebSocket in background
                    let network = self.network.clone();
                    self.runtime.spawn(async move {
                        if let Err(e) = network.connect().await {
                            tracing::error!("Failed to connect: {}", e);
                        }
                    });
                }
            }
            View::Main => {
                let open_settings = self.main_view.show(
                    ctx,
                    &self.state,
                    &self.network,
                    &self.runtime,
                );
                if open_settings {
                    self.view = View::Settings;
                }
            }
            View::Settings => {
                let close = self.settings_view.show(ctx, &self.state, &self.runtime);
                if close {
                    self.view = View::Main;
                }
            }
        }
    }
}
