use crate::network::{GifSearchResponse, NetworkClient, TenorGif};
use crate::state::AppState;
use eframe::egui;
use std::collections::HashMap;
use std::time::Instant;
use tokio::runtime::Runtime;

/// Debounce delay for search queries (milliseconds)
const SEARCH_DEBOUNCE_MS: u64 = 300;

/// Number of columns in the GIF grid
const GRID_COLUMNS: usize = 4;

/// GIF thumbnail size in pixels
const THUMBNAIL_SIZE: f32 = 80.0;

/// Maximum GIFs to display
const MAX_GIFS: u32 = 24;

pub struct GifPicker {
    is_open: bool,
    search_query: String,
    results: Vec<TenorGif>,
    loading: bool,
    error_message: Option<String>,
    /// Texture cache for GIF previews
    gif_textures: HashMap<String, egui::TextureHandle>,
    /// Track which GIFs are being loaded
    loading_gifs: std::collections::HashSet<String>,
    /// Last search time for debouncing
    last_search_time: Option<Instant>,
    /// Last search query that was executed
    last_executed_query: String,
    /// Whether trending GIFs have been loaded
    trending_loaded: bool,
}

impl Default for GifPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl GifPicker {
    pub fn new() -> Self {
        Self {
            is_open: false,
            search_query: String::new(),
            results: Vec::new(),
            loading: false,
            error_message: None,
            gif_textures: HashMap::new(),
            loading_gifs: std::collections::HashSet::new(),
            last_search_time: None,
            last_executed_query: String::new(),
            trending_loaded: false,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.search_query.clear();
        self.error_message = None;
        // Reset trending loaded so we fetch fresh trending on open
        self.trending_loaded = false;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Show the GIF picker popup
    /// Returns Some(gif_url) if the user selected a GIF
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        input_rect: egui::Rect,
        network: &NetworkClient,
        state: &AppState,
        runtime: &Runtime,
    ) -> Option<String> {
        if !self.is_open {
            return None;
        }

        // Handle escape to close
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if escape {
            self.close();
            return None;
        }

        // Trigger search or trending fetch
        self.handle_search(network, state, runtime);

        let mut selected_gif_url: Option<String> = None;

        // Calculate popup position (above the input area)
        let popup_width = 360.0;
        let popup_height = 400.0;
        let popup_pos = egui::pos2(
            input_rect.left(),
            input_rect.top() - popup_height - 8.0,
        );

        egui::Area::new(egui::Id::new("gif_picker_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup_pos)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(super::theme::BG_ELEVATED)
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .stroke(egui::Stroke::new(1.0, super::theme::BG_ACCENT))
                    .shadow(egui::epaint::Shadow {
                        offset: egui::vec2(0.0, 4.0),
                        blur: 16.0,
                        spread: 0.0,
                        color: egui::Color32::from_black_alpha(80),
                    })
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(popup_width, popup_height));
                        ui.set_max_size(egui::vec2(popup_width, popup_height));

                        // Header with close button
                        ui.horizontal(|ui| {
                            ui.heading(egui::RichText::new("GIFs").size(16.0));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("✕").clicked() {
                                    self.close();
                                }
                            });
                        });

                        ui.add_space(8.0);

                        // Search input
                        let search_response = ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .hint_text("Search Tenor...")
                                .desired_width(popup_width - 24.0)
                        );

                        // Focus search input on first show
                        if search_response.gained_focus() || self.results.is_empty() {
                            search_response.request_focus();
                        }

                        ui.add_space(8.0);

                        // Content area (scrollable)
                        egui::ScrollArea::vertical()
                            .max_height(popup_height - 100.0)
                            .show(ui, |ui| {
                                if self.loading && self.results.is_empty() {
                                    ui.centered_and_justified(|ui| {
                                        ui.spinner();
                                    });
                                } else if let Some(error) = &self.error_message {
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            egui::RichText::new(error)
                                                .color(super::theme::TEXT_MUTED)
                                        );
                                    });
                                } else if self.results.is_empty() {
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            egui::RichText::new("No GIFs found")
                                                .color(super::theme::TEXT_MUTED)
                                        );
                                    });
                                } else {
                                    // GIF grid
                                    selected_gif_url = self.render_gif_grid(ui, ctx, network, state, runtime);
                                }
                            });

                        // Footer with Tenor attribution
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Powered by Tenor")
                                    .size(10.0)
                                    .color(super::theme::TEXT_MUTED)
                            );
                        });
                    });
            });

        // Request repaint for loading states
        if self.loading || !self.loading_gifs.is_empty() {
            ctx.request_repaint();
        }

        selected_gif_url
    }

    fn handle_search(&mut self, network: &NetworkClient, state: &AppState, runtime: &Runtime) {
        let query = self.search_query.trim().to_string();

        // Debounce search queries
        let should_search = if query.is_empty() {
            // Load trending if not yet loaded
            !self.trending_loaded && !self.loading
        } else {
            // Check debounce
            let now = Instant::now();
            let debounced = self.last_search_time
                .map(|t| now.duration_since(t).as_millis() >= SEARCH_DEBOUNCE_MS as u128)
                .unwrap_or(true);

            debounced && query != self.last_executed_query && !self.loading
        };

        if should_search {
            self.loading = true;
            self.error_message = None;
            self.last_search_time = Some(Instant::now());
            self.last_executed_query = query.clone();

            let network = network.clone();
            let state_clone = state.clone();

            if query.is_empty() {
                self.trending_loaded = true;
                runtime.spawn(async move {
                    match network.get_trending_gifs(MAX_GIFS).await {
                        Ok(response) => {
                            state_clone.set_gif_search_results(response.results).await;
                        }
                        Err(e) => {
                            tracing::error!("Failed to fetch trending GIFs: {}", e);
                            state_clone.set_gif_search_error(format!("Failed to load GIFs")).await;
                        }
                    }
                });
            } else {
                runtime.spawn(async move {
                    match network.search_gifs(&query, MAX_GIFS).await {
                        Ok(response) => {
                            state_clone.set_gif_search_results(response.results).await;
                        }
                        Err(e) => {
                            tracing::error!("Failed to search GIFs: {}", e);
                            state_clone.set_gif_search_error(format!("Failed to search GIFs")).await;
                        }
                    }
                });
            }
        }

        // Check for results from state
        if let Some((results, error)) = runtime.block_on(state.get_gif_search_results()) {
            self.loading = false;
            if let Some(err) = error {
                self.error_message = Some(err);
            } else {
                self.results = results;
            }
        }
    }

    fn render_gif_grid(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        network: &NetworkClient,
        state: &AppState,
        runtime: &Runtime,
    ) -> Option<String> {
        let mut selected_url: Option<String> = None;

        // Calculate grid layout
        let available_width = ui.available_width();
        let spacing = 4.0;
        let cell_size = (available_width - (spacing * (GRID_COLUMNS as f32 - 1.0))) / GRID_COLUMNS as f32;

        // Extract GIF data first to avoid borrowing issues
        let gif_data: Vec<_> = self.results.iter().map(|gif| {
            let preview_url = gif.media_formats.nanogif
                .as_ref()
                .or(gif.media_formats.tinygif.as_ref())
                .map(|m| m.url.clone());
            let full_url = gif.media_formats.gif
                .as_ref()
                .or(gif.media_formats.tinygif.as_ref())
                .map(|m| m.url.clone());
            let title = gif.title.clone();
            (preview_url, full_url, title)
        }).collect();

        egui::Grid::new("gif_grid")
            .num_columns(GRID_COLUMNS)
            .spacing([spacing, spacing])
            .show(ui, |ui| {
                for (i, (preview_url, full_url, title)) in gif_data.iter().enumerate() {
                    if let (Some(preview_url), Some(full_url)) = (preview_url, full_url) {
                        let response = self.render_gif_thumbnail(
                            ui,
                            ctx,
                            preview_url,
                            cell_size,
                            network,
                            state,
                            runtime,
                        );

                        if response.clicked() {
                            selected_url = Some(full_url.clone());
                        }

                        // Show tooltip with title
                        if !title.is_empty() {
                            response.on_hover_text(title);
                        }
                    }

                    // New row after GRID_COLUMNS items
                    if (i + 1) % GRID_COLUMNS == 0 {
                        ui.end_row();
                    }
                }
            });

        selected_url
    }

    fn render_gif_thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        url: &str,
        size: f32,
        network: &NetworkClient,
        state: &AppState,
        runtime: &Runtime,
    ) -> egui::Response {
        // Check if we have the texture cached
        if let Some(texture) = self.gif_textures.get(url) {
            // Render the cached texture
            let image = egui::Image::new(texture)
                .fit_to_exact_size(egui::vec2(size, size))
                .rounding(4.0);

            ui.add(image)
                .interact(egui::Sense::click())
        } else {
            // Check if image is in the global cache
            if let Some(cached_data) = state.get_image_sync(url) {
                let (rgba, width, height) = cached_data.as_ref();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [*width as usize, *height as usize],
                    rgba,
                );
                let texture = ctx.load_texture(
                    url,
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                self.gif_textures.insert(url.to_string(), texture.clone());

                let image = egui::Image::new(&texture)
                    .fit_to_exact_size(egui::vec2(size, size))
                    .rounding(4.0);

                ui.add(image)
                    .interact(egui::Sense::click())
            } else {
                // Start loading if not already loading
                if !self.loading_gifs.contains(url) {
                    self.loading_gifs.insert(url.to_string());
                    let url_clone = url.to_string();
                    let network = network.clone();
                    let state_clone = state.clone();
                    runtime.spawn(async move {
                        match network.fetch_image(&url_clone).await {
                            Ok((rgba, width, height)) => {
                                state_clone.set_image(url_clone.clone(), rgba, width, height).await;
                            }
                            Err(e) => {
                                tracing::error!("Failed to load GIF thumbnail: {}", e);
                                state_clone.mark_image_failed(&url_clone).await;
                            }
                        }
                    });
                }

                // Show placeholder while loading
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(size, size),
                    egui::Sense::click(),
                );

                painter.rect_filled(
                    response.rect,
                    4.0,
                    super::theme::BG_ACCENT,
                );

                // Show spinner in center
                let center = response.rect.center();
                painter.circle_stroke(
                    center,
                    8.0,
                    egui::Stroke::new(2.0, super::theme::TEXT_MUTED),
                );

                response
            }
        }
    }
}
