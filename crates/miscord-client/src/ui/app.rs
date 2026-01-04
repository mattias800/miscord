use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;

use super::login::LoginView;
use super::main_view::MainView;

pub struct MiscordApp {
    state: AppState,
    network: NetworkClient,
    runtime: tokio::runtime::Runtime,
    view: View,
    login_view: LoginView,
    main_view: MainView,
}

enum View {
    Login,
    Main,
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
        }
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
                if let Some((token, user)) =
                    self.login_view
                        .show(ctx, &self.network, &self.runtime)
                {
                    let state = self.state.clone();
                    self.runtime.block_on(async {
                        state.set_auth(token, user).await;
                    });
                    self.view = View::Main;

                    // Connect WebSocket and load initial data
                    let network = self.network.clone();
                    self.runtime.spawn(async move {
                        if let Err(e) = network.connect().await {
                            tracing::error!("Failed to connect: {}", e);
                        }
                    });
                }
            }
            View::Main => {
                self.main_view.show(
                    ctx,
                    &self.state,
                    &self.network,
                    &self.runtime,
                );
            }
        }
    }
}
