use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;

use super::login::LoginView;
use super::main_view::MainView;
use super::settings::SettingsView;

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
        // Set up custom fonts and style
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        cc.egui_ctx.set_style(style);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        let state = AppState::new();
        let network = NetworkClient::new(state.clone());

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
                        // Load servers before switching to Main view
                        if let Err(e) = network.load_servers().await {
                            tracing::error!("Failed to load servers: {}", e);
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
