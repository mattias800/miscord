use eframe::egui;

use crate::network::NetworkClient;
use crate::state::{AppState, PersistentSettings};

use super::login::LoginView;
use super::main_view::MainView;
use super::settings::SettingsView;
use super::quick_switcher::{QuickSwitcher, SwitcherItem};
use super::message_search::{MessageSearch, SearchSelection};
use super::theme;

pub struct MiscordApp {
    state: AppState,
    network: NetworkClient,
    runtime: tokio::runtime::Runtime,
    view: View,
    login_view: LoginView,
    main_view: MainView,
    settings_view: SettingsView,
    quick_switcher: QuickSwitcher,
    message_search: MessageSearch,
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

        // Increase global font sizes for better readability
        use egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(13.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
        ].into();

        // Better spacing
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.window_margin = egui::Margin::same(12.0);
        style.spacing.indent = 18.0;

        // Apply Discord colors to visuals
        let visuals = &mut style.visuals;
        visuals.dark_mode = true;
        visuals.panel_fill = theme::BG_SECONDARY;
        visuals.window_fill = theme::BG_PRIMARY;
        visuals.extreme_bg_color = theme::BG_TERTIARY;
        visuals.faint_bg_color = theme::BG_ACCENT;

        // Widget styling with better rounding
        visuals.widgets.noninteractive.bg_fill = theme::BG_PRIMARY;
        visuals.widgets.noninteractive.rounding = egui::Rounding::same(4.0);
        visuals.widgets.inactive.bg_fill = theme::BG_ACCENT;
        visuals.widgets.inactive.rounding = egui::Rounding::same(4.0);
        visuals.widgets.hovered.bg_fill = theme::BG_ELEVATED;
        visuals.widgets.hovered.rounding = egui::Rounding::same(4.0);
        visuals.widgets.active.bg_fill = theme::BLURPLE;
        visuals.widgets.active.rounding = egui::Rounding::same(4.0);

        // Text colors
        visuals.widgets.noninteractive.fg_stroke.color = theme::TEXT_NORMAL;
        visuals.widgets.inactive.fg_stroke.color = theme::TEXT_NORMAL;
        visuals.widgets.hovered.fg_stroke.color = theme::TEXT_NORMAL;
        visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;

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
            quick_switcher: QuickSwitcher::new(),
            message_search: MessageSearch::new(),
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

    /// Handle global keyboard shortcuts
    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        let modifiers = if cfg!(target_os = "macos") {
            egui::Modifiers::MAC_CMD
        } else {
            egui::Modifiers::CTRL
        };

        // Cmd+T (Mac) or Ctrl+T (Windows/Linux) to toggle quick switcher
        let quick_switch = ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::T));

        if quick_switch {
            // Close message search if open
            if self.message_search.is_open() {
                self.message_search.close();
            }
            if self.quick_switcher.is_open() {
                self.quick_switcher.close();
            } else {
                self.quick_switcher.open();
            }
        }

        // Cmd+F (Mac) or Ctrl+F (Windows/Linux) to toggle message search
        let search = ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::F));

        if search {
            // Close quick switcher if open
            if self.quick_switcher.is_open() {
                self.quick_switcher.close();
            }
            if self.message_search.is_open() {
                self.message_search.close();
            } else {
                self.message_search.open();
            }
        }
    }

    /// Handle selection from quick switcher
    fn handle_switcher_selection(&mut self, item: SwitcherItem) {
        match item {
            SwitcherItem::Channel { id, community_id, .. } => {
                let state = self.state.clone();
                let network = self.network.clone();

                self.runtime.spawn(async move {
                    // Select community if different
                    {
                        let s = state.read().await;
                        if s.current_community_id != Some(community_id) {
                            drop(s);
                            state.select_community(community_id).await;

                            // Load channels for the new community
                            if let Ok(channels) = network.get_channels(community_id).await {
                                state.set_channels(channels).await;
                            }
                        }
                    }

                    // Select the channel
                    state.select_channel(id).await;
                    state.mark_channel_read(id).await;
                    let _ = network.mark_channel_read(id).await;

                    // Load messages
                    if let Ok(messages) = network.get_messages(id, None).await {
                        let mut s = state.write().await;
                        s.messages.insert(id, messages);
                    }

                    // Subscribe to channel
                    network.subscribe_channel(id).await;
                });
            }
            SwitcherItem::User { id: user_id, .. } => {
                let state = self.state.clone();
                let network = self.network.clone();

                self.runtime.spawn(async move {
                    // Create or get the DM channel with this user
                    match network.create_or_get_dm(user_id).await {
                        Ok(dm_channel) => {
                            // Clear community selection for DMs
                            {
                                let mut s = state.write().await;
                                s.current_community_id = None;
                            }

                            // Add channel to state
                            {
                                let mut s = state.write().await;
                                s.channels.insert(dm_channel.id, dm_channel.clone());
                            }

                            // Select the DM channel
                            state.select_channel(dm_channel.id).await;
                            state.mark_channel_read(dm_channel.id).await;
                            let _ = network.mark_channel_read(dm_channel.id).await;

                            // Load messages
                            if let Ok(messages) = network.get_messages(dm_channel.id, None).await {
                                let mut s = state.write().await;
                                s.messages.insert(dm_channel.id, messages);
                            }

                            // Subscribe to channel
                            network.subscribe_channel(dm_channel.id).await;
                        }
                        Err(e) => {
                            tracing::error!("Failed to create/get DM channel: {}", e);
                        }
                    }
                });
            }
        }
    }

    /// Handle selection from message search
    fn handle_search_selection(&mut self, selection: SearchSelection) {
        let state = self.state.clone();
        let network = self.network.clone();
        let message_id = selection.message_id;

        self.runtime.spawn(async move {
            // Switch community if needed, or clear for DMs
            if let Some(community_id) = selection.community_id {
                let needs_switch = {
                    let s = state.read().await;
                    s.current_community_id != Some(community_id)
                };

                if needs_switch {
                    state.select_community(community_id).await;
                    if let Ok(channels) = network.get_channels(community_id).await {
                        state.set_channels(channels).await;
                    }
                }
            } else {
                // DM message - clear community selection
                let mut s = state.write().await;
                s.current_community_id = None;
            }

            // Select the channel
            state.select_channel(selection.channel_id).await;
            state.mark_channel_read(selection.channel_id).await;
            let _ = network.mark_channel_read(selection.channel_id).await;

            // Load messages for the channel
            if let Ok(messages) = network.get_messages(selection.channel_id, None).await {
                let mut s = state.write().await;
                s.messages.insert(selection.channel_id, messages);
            }

            // Set scroll target message
            {
                let mut s = state.write().await;
                s.scroll_to_message_id = Some(message_id);
            }

            // Subscribe to channel
            network.subscribe_channel(selection.channel_id).await;
        });
    }
}

impl eframe::App for MiscordApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request continuous repainting for real-time updates
        ctx.request_repaint();

        // Handle global keyboard shortcuts FIRST (before any view handles input)
        self.handle_global_shortcuts(ctx);

        match &self.view {
            View::Login => {
                // Try session restore first (saved token from previous login)
                // Then try auto-login from environment variables
                // Finally show the login UI
                let login_result = if self.login_view.should_restore_session() {
                    self.login_view
                        .try_restore_session(&self.network, &self.runtime)
                } else if self.login_view.should_auto_login() {
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

                // Show quick switcher modal on top of main view
                if let Some(item) = self.quick_switcher.show(ctx, &self.state, &self.runtime) {
                    self.handle_switcher_selection(item);
                }

                // Show message search modal on top of main view
                if let Some(selection) = self.message_search.show(ctx, &self.state, &self.network, &self.runtime) {
                    self.handle_search_selection(selection);
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
