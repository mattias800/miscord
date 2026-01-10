//! Message Search - Cmd+F/Ctrl+F to search messages across channels

use eframe::egui;
use uuid::Uuid;

use crate::network::{MessageSearchResult, NetworkClient};
use crate::state::AppState;
use super::theme;

/// Result of selecting a search result - navigate to this message
#[derive(Debug, Clone)]
pub struct SearchSelection {
    pub message_id: Uuid,
    pub channel_id: Uuid,
    pub community_id: Option<Uuid>,
}

/// Message search modal
pub struct MessageSearch {
    is_open: bool,
    search_query: String,
    selected_index: usize,
    /// Search results from server
    results: Vec<MessageSearchResult>,
    /// Whether to request focus on the search input
    request_focus: bool,
    /// Whether a search is in progress
    is_searching: bool,
    /// Last query that was searched (to avoid duplicate searches)
    last_searched_query: String,
}

impl MessageSearch {
    pub fn new() -> Self {
        Self {
            is_open: false,
            search_query: String::new(),
            selected_index: 0,
            results: Vec::new(),
            request_focus: false,
            is_searching: false,
            last_searched_query: String::new(),
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.search_query.clear();
        self.selected_index = 0;
        self.results.clear();
        self.request_focus = true;
        self.is_searching = false;
        self.last_searched_query.clear();
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.search_query.clear();
        self.selected_index = 0;
        self.results.clear();
        self.is_searching = false;
        self.last_searched_query.clear();
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Set search results (called from async search task)
    pub fn set_results(&mut self, results: Vec<MessageSearchResult>) {
        self.results = results;
        self.is_searching = false;
        self.selected_index = 0;
    }

    /// Show the message search modal
    /// Returns the selected message if the user made a selection
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        state: &AppState,
        network: &NetworkClient,
        runtime: &tokio::runtime::Runtime,
    ) -> Option<SearchSelection> {
        if !self.is_open {
            return None;
        }

        let mut selected_item: Option<SearchSelection> = None;
        let mut should_close = false;

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
            let result = &self.results[self.selected_index];
            selected_item = Some(SearchSelection {
                message_id: result.message.id,
                channel_id: result.message.channel_id,
                community_id: runtime.block_on(async {
                    let s = state.read().await;
                    s.channels.get(&result.message.channel_id)
                        .and_then(|c| c.community_id)
                }),
            });
            should_close = true;
        }

        // Trigger search if query changed
        let query = self.search_query.trim().to_string();
        if !query.is_empty() && query != self.last_searched_query && !self.is_searching {
            self.is_searching = true;
            self.last_searched_query = query.clone();

            // Get current community for scoped search
            let community_id = runtime.block_on(async {
                state.read().await.current_community_id
            });

            let network = network.clone();
            let ctx = ctx.clone();

            // We need to store results somewhere accessible
            // For simplicity, we'll do a blocking search here
            // In production, you'd want async with state updates
            let results = runtime.block_on(async {
                network.search_messages(&query, community_id).await
            });

            match results {
                Ok(results) => {
                    self.results = results;
                    self.is_searching = false;
                    self.selected_index = 0;
                }
                Err(e) => {
                    tracing::warn!("Search failed: {}", e);
                    self.results.clear();
                    self.is_searching = false;
                }
            }
            ctx.request_repaint();
        }

        // Clear results if query is empty
        if query.is_empty() && !self.results.is_empty() {
            self.results.clear();
            self.last_searched_query.clear();
        }

        // Render backdrop (lower order)
        egui::Area::new(egui::Id::new("message_search_backdrop"))
            .order(egui::Order::Middle)
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

        // Render modal (higher order, above backdrop)
        egui::Window::new("Message Search")
            .order(egui::Order::Foreground)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .fixed_size(egui::vec2(600.0, 450.0))
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
                ui.set_min_size(egui::vec2(600.0, 450.0));

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
                                egui::RichText::new("Search messages...")
                                    .size(14.0)
                                    .color(theme::TEXT_MUTED)
                            );
                            if self.is_searching {
                                ui.spinner();
                            }
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
                    });

                ui.add_space(4.0);

                // Results area
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());

                        if self.search_query.trim().is_empty() {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("Type to search messages")
                                        .size(14.0)
                                        .color(theme::TEXT_MUTED)
                                );
                            });
                        } else if self.is_searching {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Searching...")
                                        .size(14.0)
                                        .color(theme::TEXT_MUTED)
                                );
                            });
                        } else if self.results.is_empty() {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new("No messages found")
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
                                    egui::RichText::new(format!("{} results", self.results.len()))
                                        .size(12.0)
                                        .color(theme::TEXT_MUTED)
                                );
                            });
                            ui.add_space(4.0);

                            for (idx, result) in self.results.iter().enumerate() {
                                let is_selected = idx == self.selected_index;

                                let response = self.render_message_result(
                                    ui,
                                    result,
                                    is_selected,
                                    &self.search_query,
                                );

                                if response.clicked() {
                                    selected_item = Some(SearchSelection {
                                        message_id: result.message.id,
                                        channel_id: result.message.channel_id,
                                        community_id: runtime.block_on(async {
                                            let s = state.read().await;
                                            s.channels.get(&result.message.channel_id)
                                                .and_then(|c| c.community_id)
                                        }),
                                    });
                                    should_close = true;
                                }

                                if response.hovered() {
                                    self.selected_index = idx;
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
                            self.render_hint(ui, "Enter", "go to message");
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

    fn render_message_result(
        &self,
        ui: &mut egui::Ui,
        result: &MessageSearchResult,
        is_selected: bool,
        query: &str,
    ) -> egui::Response {
        let bg_color = if is_selected {
            theme::BLURPLE_DARK
        } else {
            egui::Color32::TRANSPARENT
        };

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 70.0),
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

            let content_rect = rect.shrink2(egui::vec2(16.0, 8.0));

            // Channel and community info (top line)
            let channel_text = format!("#{} • {}", result.channel_name, result.community_name);
            let channel_pos = content_rect.left_top();
            ui.painter().text(
                channel_pos,
                egui::Align2::LEFT_TOP,
                &channel_text,
                egui::FontId::proportional(12.0),
                theme::TEXT_MUTED,
            );

            // Author and timestamp (second line)
            let author_text = format!(
                "{} • {}",
                result.message.author_name,
                format_relative_time(result.message.created_at)
            );
            let author_pos = content_rect.left_top() + egui::vec2(0.0, 16.0);
            ui.painter().text(
                author_pos,
                egui::Align2::LEFT_TOP,
                &author_text,
                egui::FontId::proportional(12.0),
                if is_selected { theme::TEXT_NORMAL } else { theme::TEXT_LINK },
            );

            // Message content preview (bottom line, truncated)
            let content = truncate_content(&result.message.content, 100);
            let content_pos = content_rect.left_top() + egui::vec2(0.0, 34.0);

            // Simple text without highlighting for now
            ui.painter().text(
                content_pos,
                egui::Align2::LEFT_TOP,
                &content,
                egui::FontId::proportional(14.0),
                if is_selected { theme::TEXT_BRIGHT } else { theme::TEXT_NORMAL },
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
}

impl Default for MessageSearch {
    fn default() -> Self {
        Self::new()
    }
}

/// Format timestamp as relative time
fn format_relative_time(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration.num_seconds() < 60 {
        "Just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() == 1 {
        "Yesterday".to_string()
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else {
        timestamp.format("%b %d, %Y").to_string()
    }
}

/// Truncate content for preview
fn truncate_content(content: &str, max_len: usize) -> String {
    // Remove newlines
    let content = content.replace('\n', " ");
    if content.len() <= max_len {
        content
    } else {
        format!("{}...", &content[..max_len])
    }
}
