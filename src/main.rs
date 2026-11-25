// En release no abrimos una consola negra junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod android;
mod app;
mod audio;
mod i18n;
mod logs;
mod paths;
mod recorder;
mod setup;
mod video;
mod window;

fn main() -> eframe::Result<()> {
    paths::migrate_legacy_data();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Reflex")
            .with_inner_size([440.0, 860.0])
            .with_min_inner_size([360.0, 560.0])
            .with_transparent(true)
            .with_decorations(false)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("icono válido")),
        // glow: el renderizador que permite ventana transparente en Windows.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Reflex",
        options,
        Box::new(|cc| Ok(Box::new(app::ReflexApp::new(cc)))),
    )
}
