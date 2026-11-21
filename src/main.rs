#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // pas de console en release

mod api;
mod app;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Marstek LAN")
            .with_inner_size([720.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native("Marstek LAN", options, Box::new(|cc| Ok(Box::new(app::MarstekApp::new(cc)))))
}
