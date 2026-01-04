use eframe::egui;
use uuid::Uuid;

use crate::network::NetworkClient;
use crate::state::AppState;

pub struct ServerList {
    show_create_dialog: bool,
    new_server_name: String,
    show_join_dialog: bool,
    invite_code: String,
}

impl ServerList {
    pub fn new() -> Self {
        Self {
            show_create_dialog: false,
            new_server_name: String::new(),
            show_join_dialog: false,
            invite_code: String::new(),
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);

            // DMs button
            let dm_button = egui::Button::new("DMs")
                .min_size(egui::vec2(48.0, 48.0));

            if ui.add(dm_button).clicked() {
                // TODO: Show DMs
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // Server list
            let servers = runtime.block_on(async {
                let s = state.read().await;
                s.servers.values().cloned().collect::<Vec<_>>()
            });

            let current_server = runtime.block_on(async {
                state.read().await.current_server_id
            });

            for server in servers {
                let is_selected = current_server == Some(server.id);

                let button = egui::Button::new(&server.name[..1].to_uppercase())
                    .min_size(egui::vec2(48.0, 48.0))
                    .fill(if is_selected {
                        egui::Color32::from_rgb(88, 101, 242)
                    } else {
                        egui::Color32::from_rgb(54, 57, 63)
                    });

                let response = ui.add(button);

                if response.clicked() {
                    let state = state.clone();
                    let network = network.clone();
                    let server_id = server.id;

                    runtime.spawn(async move {
                        state.select_server(server_id).await;

                        // Load channels for this server
                        if let Ok(channels) = network.get_channels(server_id).await {
                            state.set_channels(channels).await;
                        }
                    });
                }

                response.on_hover_text(&server.name);
            }

            ui.add_space(8.0);

            // Add server button
            let add_button = egui::Button::new("+")
                .min_size(egui::vec2(48.0, 48.0));

            if ui.add(add_button).on_hover_text("Add a Server").clicked() {
                self.show_create_dialog = true;
            }
        });

        // Create server dialog
        if self.show_create_dialog {
            egui::Window::new("Create Server")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.new_server_name);
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            let name = self.new_server_name.clone();
                            let network = network.clone();
                            let state = state.clone();

                            runtime.spawn(async move {
                                if let Ok(server) = network.create_server(&name).await {
                                    let mut s = state.write().await;
                                    s.servers.insert(server.id, server);
                                }
                            });

                            self.new_server_name.clear();
                            self.show_create_dialog = false;
                        }

                        if ui.button("Cancel").clicked() {
                            self.new_server_name.clear();
                            self.show_create_dialog = false;
                        }
                    });

                    ui.add_space(10.0);

                    if ui.button("Join with Invite Code").clicked() {
                        self.show_create_dialog = false;
                        self.show_join_dialog = true;
                    }
                });
        }

        // Join server dialog
        if self.show_join_dialog {
            egui::Window::new("Join Server")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Invite Code:");
                        ui.text_edit_singleline(&mut self.invite_code);
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Join").clicked() {
                            let code = self.invite_code.clone();
                            let network = network.clone();
                            let state = state.clone();

                            runtime.spawn(async move {
                                if let Ok(server) = network.join_server(&code).await {
                                    let mut s = state.write().await;
                                    s.servers.insert(server.id, server);
                                }
                            });

                            self.invite_code.clear();
                            self.show_join_dialog = false;
                        }

                        if ui.button("Cancel").clicked() {
                            self.invite_code.clear();
                            self.show_join_dialog = false;
                        }
                    });
                });
        }
    }
}

impl Default for ServerList {
    fn default() -> Self {
        Self::new()
    }
}
