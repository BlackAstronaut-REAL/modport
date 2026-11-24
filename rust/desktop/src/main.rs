mod app;
mod browser;
mod renderer;
mod resource_model;
mod tooltip;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MojModelBridge")
            .with_inner_size([1560.0, 960.0])
            .with_min_inner_size([1120.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MojModelBridge",
        options,
        Box::new(|creation_context| Ok(Box::new(app::MojModelBridgeApp::new(creation_context)))),
    )
}
