use anyhow::Result;
use eframe::egui;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod media;
mod network;
mod state;
mod ui;

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "miscord=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Miscord client...");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("Miscord"),
        ..Default::default()
    };

    eframe::run_native(
        "Miscord",
        options,
        Box::new(|cc| Ok(Box::new(ui::MiscordApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run eframe: {}", e))?;

    Ok(())
}
