use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;

pub struct VoicePanel;

impl VoicePanel {
    pub fn new() -> Self {
        Self
    }

    pub fn show_controls(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (voice_channel_id, is_muted, is_deafened, is_video, is_screen) =
            runtime.block_on(async {
                let s = state.read().await;
                (
                    s.voice_channel_id,
                    s.is_muted,
                    s.is_deafened,
                    s.is_video_enabled,
                    s.is_screen_sharing,
                )
            });

        if voice_channel_id.is_none() {
            return;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // Voice controls
        ui.horizontal(|ui| {
            // Mute button
            let mute_label = if is_muted { "🔇" } else { "🎤" };
            if ui.button(mute_label).on_hover_text("Toggle Mute").clicked() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    let new_muted = !is_muted;
                    if network.update_voice_state(Some(new_muted), None, None, None).await.is_ok() {
                        let mut s = state.write().await;
                        s.is_muted = new_muted;
                    }
                });
            }

            // Deafen button
            let deafen_label = if is_deafened { "🔇" } else { "🔊" };
            if ui.button(deafen_label).on_hover_text("Toggle Deafen").clicked() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    let new_deafened = !is_deafened;
                    if network.update_voice_state(None, Some(new_deafened), None, None).await.is_ok() {
                        let mut s = state.write().await;
                        s.is_deafened = new_deafened;
                    }
                });
            }

            // Video button
            let video_label = if is_video { "📹" } else { "📷" };
            if ui.button(video_label).on_hover_text("Toggle Video").clicked() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    let new_video = !is_video;
                    if network.update_voice_state(None, None, Some(new_video), None).await.is_ok() {
                        let mut s = state.write().await;
                        s.is_video_enabled = new_video;
                    }
                });
            }

            // Screen share button
            let screen_label = if is_screen { "🖥️" } else { "💻" };
            if ui.button(screen_label).on_hover_text("Toggle Screen Share").clicked() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    let new_screen = !is_screen;
                    if network.update_voice_state(None, None, None, Some(new_screen)).await.is_ok() {
                        let mut s = state.write().await;
                        s.is_screen_sharing = new_screen;
                    }
                });
            }

            // Disconnect button
            if ui.button("❌").on_hover_text("Disconnect").clicked() {
                let state = state.clone();
                let network = network.clone();
                runtime.spawn(async move {
                    network.leave_voice().await;
                    state.leave_voice().await;
                });
            }
        });
    }

    pub fn show_participants(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) {
        ui.heading("Voice Chat");
        ui.separator();

        let participants = runtime.block_on(async {
            state.read().await.voice_participants.values().cloned().collect::<Vec<_>>()
        });

        for participant in participants {
            ui.horizontal(|ui| {
                // Speaking indicator
                if participant.is_speaking {
                    ui.label(egui::RichText::new("🗣️").color(egui::Color32::GREEN));
                } else {
                    ui.label("  ");
                }

                // Username
                ui.label(&participant.username);

                // Status icons
                if participant.is_muted {
                    ui.label("🔇");
                }
                if participant.is_deafened {
                    ui.label("🔇");
                }
                if participant.is_video_enabled {
                    ui.label("📹");
                }
                if participant.is_screen_sharing {
                    ui.label("🖥️");
                }
            });
        }
    }
}

impl Default for VoicePanel {
    fn default() -> Self {
        Self::new()
    }
}
