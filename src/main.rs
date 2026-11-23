#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod db;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        persist_window: true,
        viewport: egui::ViewportBuilder::default()
            .with_maximized(true)
            .with_inner_size([920.0, 620.0])
            .with_min_inner_size([500.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Journal",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            Ok(Box::new(app::Journal::new()?))
        }),
    )
}
