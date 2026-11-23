//! Controls usable in any egui app. No dependency on the device backend.
pub use egui;
pub mod accessibility;
pub mod comp;
pub mod timeline;
pub mod wave_edit;
use soniclaw_core::Parameter;
pub mod toolkit;
pub use toolkit::*;
mod clips;
pub use clips::*;
mod controls;
mod editors;
pub use controls::*;
pub use editors::*;

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
            .suffix(if parameter.has_codec() { "" } else { info.unit })
            .custom_formatter(|v, _| parameter.format_value(v as f32))
            .custom_parser(|text| parameter.parse_value(text).map(|v| v as f64))
            .logarithmic(logarithmic && info.min > 0.0)
            .step_by(if parameter.step_count() > 0 { 1. } else { 0. }),
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
        toolkit::theme_color(toolkit::current_theme(ui.ctx()).danger)
    } else {
        toolkit::accent(ui)
    };
    ui.add(
        egui::ProgressBar::new(((db + 60.0) / 60.0).clamp(0.0, 1.0))
            .fill(color)
            .text(format!("{db:.1} dBFS")),
    );
}
pub fn apply_theme(context: &egui::Context) {
    Theme::dark().apply(context).expect("built-in theme valid");
}

/// Automatically select a typed enum/bool control or a numeric slider from shared parameter metadata.
pub fn parameter_control(ui: &mut egui::Ui, p: &Parameter) -> egui::Response {
    let info = p.info();
    if p.is_enum() && p.step_count() == 1 {
        let mut value = p.get() >= 0.5;
        let response = ui.checkbox(&mut value, info.name);
        if response.changed() {
            let _ = p.set(u8::from(value) as f32);
        }
        return response;
    }
    if p.is_enum() {
        let mut value = p.get();
        let mut changed = false;
        let result = egui::ComboBox::from_id_salt(info.id)
            .selected_text(p.format_value(value))
            .show_ui(ui, |ui| {
                for i in 0..=p.step_count() {
                    let n = info.min + i as f32;
                    changed |= ui
                        .selectable_value(&mut value, n, p.format_value(n))
                        .changed();
                }
            });
        let mut response = result.response;
        if changed {
            let _ = p.set(value);
            response.mark_changed();
        }
        response.on_hover_text(info.name)
    } else {
        parameter_slider(ui, p, false)
    }
}

mod commands;
pub use commands::*;

mod mixer;
pub use mixer::*;
