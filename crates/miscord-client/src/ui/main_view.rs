use eframe::egui;

use crate::network::NetworkClient;
use crate::state::AppState;

use super::channel_list::ChannelList;
use super::chat::ChatView;
use super::community_list::CommunityList;
use super::member_list::MemberList;
use super::voice::VoicePanel;

pub struct MainView {
    community_list: CommunityList,
    channel_list: ChannelList,
    chat_view: ChatView,
    member_list: MemberList,
    voice_panel: VoicePanel,
}

impl MainView {
    pub fn new() -> Self {
        Self {
            community_list: CommunityList::new(),
            channel_list: ChannelList::new(),
            chat_view: ChatView::new(),
            member_list: MemberList::new(),
            voice_panel: VoicePanel::new(),
        }
    }

    /// Returns true if settings should be opened
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> bool {
        let mut open_settings = false;

        // Left panel - Community list
        egui::SidePanel::left("community_panel")
            .exact_width(72.0)
            .show(ctx, |ui| {
                self.community_list.show(ui, state, network, runtime);
            });

        // Channel list panel
        egui::SidePanel::left("channel_panel")
            .min_width(200.0)
            .max_width(300.0)
            .show(ctx, |ui| {
                self.channel_list.show(ui, state, network, runtime);

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    // Settings button row
                    ui.horizontal(|ui| {
                        if ui.button("\u{2699}").on_hover_text("Settings").clicked() {
                            open_settings = true;
                        }
                    });
                    ui.add_space(4.0);

                    self.voice_panel.show_controls(ui, state, network, runtime);
                });
            });

        // Right panel - Voice participants (if in voice) or Member list
        let (in_voice, has_community) = runtime.block_on(async {
            let s = state.read().await;
            (s.voice_channel_id.is_some(), s.current_community_id.is_some())
        });

        if in_voice {
            egui::SidePanel::right("voice_panel")
                .min_width(200.0)
                .max_width(300.0)
                .show(ctx, |ui| {
                    self.voice_panel.show_participants(ui, state, runtime);
                });
        } else if has_community {
            egui::SidePanel::right("member_panel")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.member_list.show(ui, state, runtime);
                    });
                });
        }

        // Main chat area
        egui::CentralPanel::default().show(ctx, |ui| {
            self.chat_view.show(ui, state, network, runtime);
        });

        open_settings
    }
}

impl Default for MainView {
    fn default() -> Self {
        Self::new()
    }
}
