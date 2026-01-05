use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;
use super::theme;

pub struct CommunityList {
    show_create_dialog: bool,
    new_community_name: String,
    show_join_dialog: bool,
    invite_code: String,
}

impl CommunityList {
    pub fn new() -> Self {
        Self {
            show_create_dialog: false,
            new_community_name: String::new(),
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

            // Community list
            let communities = runtime.block_on(async {
                let s = state.read().await;
                s.communities.values().cloned().collect::<Vec<_>>()
            });

            let current_community = runtime.block_on(async {
                state.read().await.current_community_id
            });

            for community in communities {
                let is_selected = current_community == Some(community.id);

                let button = egui::Button::new(&community.name[..1].to_uppercase())
                    .min_size(egui::vec2(48.0, 48.0))
                    .fill(if is_selected {
                        theme::BLURPLE
                    } else {
                        theme::BG_PRIMARY
                    });

                let response = ui.add(button);

                if response.clicked() {
                    let state = state.clone();
                    let network = network.clone();
                    let community_id = community.id;

                    runtime.spawn(async move {
                        state.select_community(community_id).await;

                        // Load channels for this community
                        if let Ok(channels) = network.get_channels(community_id).await {
                            state.set_channels(channels).await;
                        }

                        // Load members for this community
                        if let Ok(members) = network.get_members(community_id).await {
                            state.set_members(community_id, members).await;
                        }
                    });
                }

                response.on_hover_text(&community.name);
            }

            ui.add_space(8.0);

            // Add community button
            let add_button = egui::Button::new("+")
                .min_size(egui::vec2(48.0, 48.0));

            if ui.add(add_button).on_hover_text("Add a Community").clicked() {
                self.show_create_dialog = true;
            }
        });

        // Create community dialog
        if self.show_create_dialog {
            egui::Window::new("Create Community")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.new_community_name);
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
                            let name = self.new_community_name.clone();
                            let network = network.clone();
                            let state = state.clone();

                            runtime.spawn(async move {
                                if let Ok(community) = network.create_community(&name).await {
                                    let mut s = state.write().await;
                                    s.communities.insert(community.id, community);
                                }
                            });

                            self.new_community_name.clear();
                            self.show_create_dialog = false;
                        }

                        if ui.button("Cancel").clicked() {
                            self.new_community_name.clear();
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

        // Join community dialog
        if self.show_join_dialog {
            egui::Window::new("Join Community")
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
                                if let Ok(community) = network.join_community(&code).await {
                                    let mut s = state.write().await;
                                    s.communities.insert(community.id, community);
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

impl Default for CommunityList {
    fn default() -> Self {
        Self::new()
    }
}
