#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod batch;
mod ffmpeg;
mod model;
mod settings;
mod worker;

use eframe::egui;
use tracing_subscriber::EnvFilter;

const APP_NAME: &str = "BurnSubs";
const APP_ID: &str = "net.rambod.burnsubs";

fn main() -> eframe::Result<()> {
    initialize_logging();

    tracing::info!("Starting {APP_NAME}");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_ID)
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([860.0, 620.0])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        native_options,
        Box::new(|creation_context| Ok(Box::new(app::BurnSubsApp::new(creation_context)))),
    )
}

fn initialize_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init()
    {
        eprintln!("Failed to initialize logging: {error}");
    }
}
