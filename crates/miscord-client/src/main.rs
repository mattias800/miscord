use anyhow::Result;
use eframe::egui;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use miscord_client::ui;

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "miscord=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Get window title from environment or use default
    let window_title = std::env::var("MISCORD_WINDOW_TITLE").unwrap_or_else(|_| "Miscord".to_string());

    tracing::info!("Starting Miscord client: {}", window_title);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title(&window_title),
        ..Default::default()
    };

    eframe::run_native(
        &window_title,
        options,
        Box::new(|cc| Ok(Box::new(ui::MiscordApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run eframe: {}", e))?;

    Ok(())
}
