//! Quick Switcher - Cmd+T/Ctrl+T to quickly navigate to channels

use eframe::egui;
use uuid::Uuid;

use crate::state::AppState;
use super::theme;

/// Item that can be selected in the quick switcher
#[derive(Debug, Clone)]
pub enum SwitcherItem {
    Channel {
        id: Uuid,
        name: String,
        community_name: String,
        community_id: Uuid,
    },
}

/// Quick switcher modal for fast channel navigation
pub struct QuickSwitcher {
    is_open: bool,
    search_query: String,
    selected_index: usize,
    /// Cached results to display
    results: Vec<SwitcherItem>,
    /// Whether to request focus on the search input
    request_focus: bool,
}

impl QuickSwitcher {
    pub fn new() -> Self {
        Self {
            is_open: false,
            search_query: String::new(),
            selected_index: 0,
            results: Vec::new(),
            request_focus: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.search_query.clear();
        self.selected_index = 0;
        self.results.clear();
        self.request_focus = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.search_query.clear();
        self.selected_index = 0;
        self.results.clear();
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Show the quick switcher modal
    /// Returns the selected item if the user made a selection
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &AppState,
        runtime: &tokio::runtime::Runtime,
    ) -> Option<SwitcherItem> {
        if !self.is_open {
            return None;
        }

        let mut selected_item: Option<SwitcherItem> = None;
        let mut should_close = false;

        // Build results list based on search query
        self.update_results(state, runtime);

        // Handle keyboard navigation before rendering
        let up = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        let down = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let tab = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

        if escape {
            should_close = true;
        }

        if up && self.selected_index > 0 {
            self.selected_index -= 1;
        }

        if (down || tab) && !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1).min(self.results.len() - 1);
        }

        if enter && !self.results.is_empty() {
            selected_item = Some(self.results[self.selected_index].clone());
            should_close = true;
        }

        // Render modal
        egui::Area::new(egui::Id::new("quick_switcher_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(0.0, 0.0))
            .show(ctx, |ui| {
                let screen_rect = ui.ctx().screen_rect();

                // Semi-transparent backdrop
                ui.painter().rect_filled(
                    screen_rect,
                    egui::Rounding::ZERO,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150),
                );

                // Click on backdrop to close
                let backdrop_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if backdrop_response.clicked() {
                    should_close = true;
                }
            });

        egui::Window::new("Quick Switcher")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .fixed_size(egui::vec2(500.0, 400.0))
            .frame(egui::Frame::none()
                .fill(theme::BG_PRIMARY)
                .rounding(egui::Rounding::same(12.0))
                .shadow(egui::epaint::Shadow {
                    spread: 16.0,
                    blur: 32.0,
                    color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
                    offset: egui::vec2(0.0, 8.0),
                })
                .inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(500.0, 400.0));

                // Search input area
                egui::Frame::none()
                    .fill(theme::BG_SECONDARY)
                    .rounding(egui::Rounding {
                        nw: 12.0,
                        ne: 12.0,
                        sw: 0.0,
                        se: 0.0,
                    })
                    .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Search channels...")
                                    .size(14.0)
                                    .color(theme::TEXT_MUTED)
                            );
                        });

                        ui.add_space(8.0);

                        let text_edit = egui::TextEdit::singleline(&mut self.search_query)
                            .font(egui::TextStyle::Body)
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            .hint_text("Type to search...");

                        let response = ui.add(text_edit);

                        // Request focus on first frame
                        if self.request_focus {
                            response.request_focus();
                            self.request_focus = false;
                        }

                        // Reset selection when query changes
                        if response.changed() {
                            self.selected_index = 0;
                        }
                    });

                ui.add_space(4.0);

                // Results area
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());

                        if self.results.is_empty() {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new(if self.search_query.is_empty() {
                                        "No recent channels"
                                    } else {
                                        "No channels found"
                                    })
                                    .size(14.0)
                                    .color(theme::TEXT_MUTED)
                                );
                            });
                        } else {
                            // Section header
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(if self.search_query.is_empty() {
                                        "Recent"
                                    } else {
                                        "Channels"
                                    })
                                    .size(12.0)
                                    .color(theme::TEXT_MUTED)
                                );
                            });
                            ui.add_space(4.0);

                            for (idx, item) in self.results.iter().enumerate() {
                                let is_selected = idx == self.selected_index;

                                match item {
                                    SwitcherItem::Channel { name, community_name, .. } => {
                                        let response = self.render_channel_item(
                                            ui,
                                            name,
                                            community_name,
                                            is_selected,
                                        );

                                        if response.clicked() {
                                            selected_item = Some(item.clone());
                                            should_close = true;
                                        }

                                        if response.hovered() {
                                            self.selected_index = idx;
                                        }
                                    }
                                }
                            }
                        }

                        ui.add_space(8.0);
                    });

                // Footer with keyboard hints
                ui.add_space(4.0);
                egui::Frame::none()
                    .fill(theme::BG_SECONDARY)
                    .rounding(egui::Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 12.0,
                        se: 12.0,
                    })
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            self.render_hint(ui, "↑↓", "navigate");
                            ui.add_space(16.0);
                            self.render_hint(ui, "Enter", "select");
                            ui.add_space(16.0);
                            self.render_hint(ui, "Esc", "close");
                        });
                    });
            });

        if should_close {
            self.close();
        }

        selected_item
    }

    fn render_channel_item(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        community_name: &str,
        is_selected: bool,
    ) -> egui::Response {
        let bg_color = if is_selected {
            theme::BLURPLE_DARK
        } else {
            egui::Color32::TRANSPARENT
        };

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 40.0),
            egui::Sense::click(),
        );

        if ui.is_rect_visible(rect) {
            // Background
            let bg = if response.hovered() && !is_selected {
                theme::BG_ACCENT
            } else {
                bg_color
            };

            ui.painter().rect_filled(rect, egui::Rounding::same(4.0), bg);

            // Content
            let content_rect = rect.shrink2(egui::vec2(16.0, 0.0));

            // Channel icon (#)
            let icon_rect = egui::Rect::from_min_size(
                content_rect.left_top() + egui::vec2(0.0, 10.0),
                egui::vec2(20.0, 20.0),
            );
            ui.painter().text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                "#",
                egui::FontId::proportional(16.0),
                theme::CHANNEL_ICON,
            );

            // Channel name
            let name_pos = content_rect.left_top() + egui::vec2(28.0, 12.0);
            ui.painter().text(
                name_pos,
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(15.0),
                if is_selected { theme::TEXT_BRIGHT } else { theme::TEXT_NORMAL },
            );

            // Community name (right side)
            let community_pos = content_rect.right_top() + egui::vec2(-8.0, 12.0);
            ui.painter().text(
                community_pos,
                egui::Align2::RIGHT_CENTER,
                community_name,
                egui::FontId::proportional(13.0),
                theme::TEXT_MUTED,
            );
        }

        response
    }

    fn render_hint(&self, ui: &mut egui::Ui, key: &str, action: &str) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;

            egui::Frame::none()
                .fill(theme::BG_ACCENT)
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(key)
                            .size(11.0)
                            .color(theme::TEXT_NORMAL)
                    );
                });

            ui.label(
                egui::RichText::new(action)
                    .size(11.0)
                    .color(theme::TEXT_MUTED)
            );
        });
    }

    fn update_results(&mut self, state: &AppState, runtime: &tokio::runtime::Runtime) {
        let query = self.search_query.to_lowercase();

        runtime.block_on(async {
            let s = state.read().await;

            // Get all channels with their community info
            let mut all_channels: Vec<SwitcherItem> = s.channels.values()
                .filter(|c| c.channel_type == miscord_protocol::ChannelType::Text)
                .filter_map(|channel| {
                    let community_id = channel.community_id?;
                    let community = s.communities.get(&community_id)?;
                    Some(SwitcherItem::Channel {
                        id: channel.id,
                        name: channel.name.clone(),
                        community_name: community.name.clone(),
                        community_id,
                    })
                })
                .collect();

            if query.is_empty() {
                // Show recent channels
                let recent_ids = &s.recent_channel_ids;
                self.results = recent_ids.iter()
                    .filter_map(|id| {
                        all_channels.iter().find(|item| {
                            matches!(item, SwitcherItem::Channel { id: channel_id, .. } if channel_id == id)
                        }).cloned()
                    })
                    .collect();
            } else {
                // Fuzzy search - score and sort
                let mut scored: Vec<(SwitcherItem, i32)> = all_channels.into_iter()
                    .filter_map(|item| {
                        let score = match &item {
                            SwitcherItem::Channel { name, community_name, .. } => {
                                let name_lower = name.to_lowercase();
                                let community_lower = community_name.to_lowercase();

                                if name_lower == query {
                                    100 // Exact match
                                } else if name_lower.starts_with(&query) {
                                    80 // Starts with
                                } else if name_lower.contains(&query) {
                                    60 // Contains in name
                                } else if community_lower.contains(&query) {
                                    40 // Contains in community
                                } else {
                                    return None; // No match
                                }
                            }
                        };
                        Some((item, score))
                    })
                    .collect();

                // Sort by score descending, then by name
                scored.sort_by(|a, b| {
                    b.1.cmp(&a.1).then_with(|| {
                        let name_a = match &a.0 {
                            SwitcherItem::Channel { name, .. } => name,
                        };
                        let name_b = match &b.0 {
                            SwitcherItem::Channel { name, .. } => name,
                        };
                        name_a.cmp(name_b)
                    })
                });

                self.results = scored.into_iter().map(|(item, _)| item).collect();
            }

            // Clamp selected index
            if !self.results.is_empty() {
                self.selected_index = self.selected_index.min(self.results.len() - 1);
            }
        });
    }
}

impl Default for QuickSwitcher {
    fn default() -> Self {
        Self::new()
    }
}
