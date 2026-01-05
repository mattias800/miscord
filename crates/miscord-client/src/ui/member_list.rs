use eframe::egui;
use miscord_protocol::UserStatus;

use crate::state::AppState;
use super::theme;

pub struct MemberList;

impl MemberList {
    pub fn new() -> Self {
        Self
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) {
        let (current_community_id, members) = runtime.block_on(async {
            let s = state.read().await;
            let community_id = s.current_community_id;
            let members = community_id
                .and_then(|id| s.members.get(&id).cloned())
                .unwrap_or_default();
            (community_id, members)
        });

        if current_community_id.is_none() {
            return;
        }

        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("MEMBERS - {}", members.len()))
                        .color(theme::TEXT_MUTED)
                        .small()
                );
            });

            ui.add_space(8.0);

            // Group members by status
            let online_members: Vec<_> = members
                .iter()
                .filter(|m| matches!(m.status, UserStatus::Online | UserStatus::Idle | UserStatus::DoNotDisturb))
                .collect();

            let offline_members: Vec<_> = members
                .iter()
                .filter(|m| matches!(m.status, UserStatus::Offline | UserStatus::Invisible))
                .collect();

            // Online section
            if !online_members.is_empty() {
                ui.label(
                    egui::RichText::new(format!("ONLINE - {}", online_members.len()))
                        .color(theme::TEXT_MUTED)
                        .small()
                );
                ui.add_space(4.0);

                for member in online_members {
                    self.render_member(ui, member);
                }

                ui.add_space(8.0);
            }

            // Offline section
            if !offline_members.is_empty() {
                ui.collapsing(
                    egui::RichText::new(format!("OFFLINE - {}", offline_members.len()))
                        .color(theme::TEXT_MUTED)
                        .small(),
                    |ui| {
                        for member in offline_members {
                            self.render_member(ui, member);
                        }
                    }
                );
            }
        });
    }

    fn render_member(&self, ui: &mut egui::Ui, member: &miscord_protocol::UserData) {
        ui.horizontal(|ui| {
            // Status indicator
            let status_color = match member.status {
                UserStatus::Online => theme::GREEN,
                UserStatus::Idle => theme::YELLOW,
                UserStatus::DoNotDisturb => theme::RED,
                UserStatus::Offline | UserStatus::Invisible => theme::TEXT_MUTED,
            };

            // Avatar circle with first letter
            let initial = member.display_name.chars().next().unwrap_or('?').to_uppercase().to_string();
            let (response, painter) = ui.allocate_painter(egui::vec2(32.0, 32.0), egui::Sense::hover());
            let rect = response.rect;

            // Draw avatar background
            painter.circle_filled(rect.center(), 14.0, theme::BG_ACCENT);

            // Draw initial
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &initial,
                egui::FontId::proportional(12.0),
                theme::TEXT_NORMAL,
            );

            // Draw status dot
            let status_pos = rect.right_bottom() + egui::vec2(-4.0, -4.0);
            painter.circle_filled(status_pos, 5.0, status_color);

            ui.add_space(4.0);

            // Name and status
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(&member.display_name)
                        .color(theme::TEXT_NORMAL)
                );
                if let Some(custom_status) = &member.custom_status {
                    ui.label(
                        egui::RichText::new(custom_status)
                            .color(theme::TEXT_MUTED)
                            .small()
                    );
                }
            });
        });

        ui.add_space(2.0);
    }
}

impl Default for MemberList {
    fn default() -> Self {
        Self::new()
    }
}
