// En release no abrimos una consola negra junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod android;
mod app;
mod audio;
mod logs;
mod paths;
mod recorder;
mod setup;
mod video;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Reflex")
            .with_inner_size([1180.0, 820.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Reflex",
        options,
        Box::new(|cc| Ok(Box::new(app::ReflexApp::new(cc)))),
    )
}
