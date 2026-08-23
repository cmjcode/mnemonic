//! Entry point: initializes logging and runs the `eframe` window (§4).

mod app;
mod core;
mod i18n;
mod markdown;
mod notes;
mod ui;

fn main() -> eframe::Result<()> {
    env_logger::init();

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "LONTAR",
        native_options,
        Box::new(|_cc| Ok(Box::new(app::LontarApp::new()))),
    )
}
