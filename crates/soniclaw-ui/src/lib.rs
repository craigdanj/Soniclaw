//! Controls usable in any egui app. No dependency on the device backend.
pub use egui;
use soniclaw_core::Parameter;
mod controls;
pub use controls::*;

pub fn parameter_slider(
    ui: &mut egui::Ui,
    parameter: &Parameter,
    logarithmic: bool,
) -> egui::Response {
    let info = parameter.info();
    let mut value = parameter.get();
    let response = ui.add(
        egui::Slider::new(&mut value, info.min..=info.max)
            .text(info.name)
            .suffix(info.unit)
            .logarithmic(logarithmic && info.min > 0.0),
    );
    if response.changed() {
        let _ = parameter.set(value);
    }
    response
}
pub fn level_meter(ui: &mut egui::Ui, peak: f32) {
    let db = if peak > 0.0 {
        20.0 * peak.log10()
    } else {
        -60.0
    };
    let color = if peak >= 1.0 {
        egui::Color32::from_rgb(244, 101, 101)
    } else {
        egui::Color32::from_rgb(67, 216, 172)
    };
    ui.add(
        egui::ProgressBar::new(((db + 60.0) / 60.0).clamp(0.0, 1.0))
            .fill(color)
            .text(format!("{db:.1} dBFS")),
    );
}
pub fn apply_theme(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(20, 25, 35);
    visuals.selection.bg_fill = egui::Color32::from_rgb(28, 115, 101);
    context.set_visuals(visuals);
}
