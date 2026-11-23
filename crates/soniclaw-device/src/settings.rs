//! Reusable egui settings UI. The caller handles returned control-thread actions.
use crate::manager::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Refresh,
    Apply,
    Save,
    Load,
    Retry,
    Stop,
}
#[derive(Default)]
pub struct DeviceSettingsPanel;
impl DeviceSettingsPanel {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        draft: &mut DevicePreferences,
        catalog: &OutputInventory,
        status: &ManagerStatus,
    ) -> Option<SettingsAction> {
        let mut action = None;
        ui.heading("Audio device settings");
        ui.label(format!(
            "{:?} · {} attempts · {} reconnects",
            status.state, status.attempts, status.reconnects
        ));
        if let Some(p) = &status.active {
            ui.label(format!(
                "{} · {} · {} Hz · {} channels",
                p.host, p.output.name, p.sample_rate, p.channels
            ));
            ui.label(match p.buffer_frames{Some(n)=>format!("Requested device buffer: {n} frames ({:.2} ms); actual latency is driver dependent",n as f64/p.sample_rate as f64*1000.0),None=>"Device buffer: backend default; actual latency is driver dependent".into()});
            if p.used_fallback {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Using the OS default while the preferred device is unavailable",
                );
            }
        }
        if let Some(error) = &status.last_error {
            ui.colored_label(egui::Color32::YELLOW, error);
        }
        egui::ComboBox::from_id_salt("audio-backend")
            .selected_text(draft.host.as_deref().unwrap_or("Platform default backend"))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut draft.host, None, "Platform default backend")
                    .changed()
                {
                    draft.output = None;
                }
                for h in &catalog.hosts {
                    if ui
                        .selectable_value(&mut draft.host, Some(h.clone()), h)
                        .changed()
                    {
                        draft.output = None;
                    }
                }
            });
        ui.label("Refresh after selecting a different backend.");
        egui::ComboBox::from_id_salt("audio-output")
            .selected_text(
                draft
                    .output
                    .as_ref()
                    .map(|d| format!("{} [{}]", d.name, d.occurrence + 1))
                    .unwrap_or("OS default output".into()),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut draft.output, None, "OS default output");
                for d in &catalog.outputs {
                    ui.selectable_value(
                        &mut draft.output,
                        Some(d.selector.clone()),
                        format!("{} [{}]", d.selector.name, d.selector.occurrence + 1),
                    );
                }
            });
        ui.horizontal_wrapped(|ui| {
            let mut automatic = draft.sample_rate.is_none();
            if ui.checkbox(&mut automatic, "Default sample rate").changed() {
                draft.sample_rate = if automatic { None } else { Some(48000) };
            }
            if let Some(rate) = &mut draft.sample_rate {
                ui.add(
                    egui::DragValue::new(rate)
                        .range(8000..=384000)
                        .suffix(" Hz"),
                );
            }
            let mut automatic = draft.channels.is_none();
            if ui.checkbox(&mut automatic, "Default channels").changed() {
                draft.channels = if automatic { None } else { Some(2) };
            }
            if let Some(n) = &mut draft.channels {
                ui.add(egui::DragValue::new(n).range(1..=64).suffix(" channels"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            let mut automatic = draft.buffer_frames.is_none();
            if ui.checkbox(&mut automatic, "Backend buffer size").changed() {
                draft.buffer_frames = if automatic { None } else { Some(256) };
            }
            if let Some(n) = &mut draft.buffer_frames {
                ui.add(
                    egui::DragValue::new(n)
                        .range(16..=65536)
                        .suffix(" frames requested"),
                );
            }
            ui.add(
                egui::DragValue::new(&mut draft.max_internal_frames)
                    .range(1..=8192)
                    .prefix("DSP chunk: "),
            );
        });
        let mut duplex = draft.duplex.is_some();
        if ui
            .checkbox(&mut duplex, "Unified input/output and recording")
            .changed()
        {
            draft.duplex = if duplex {
                Some(DuplexRoute::default())
            } else {
                None
            };
        }
        if let Some(route) = &mut draft.duplex {
            let mut selection = (route.use_default_input, route.input.clone());
            egui::ComboBox::from_id_salt("audio-input")
                .selected_text(
                    route
                        .input
                        .as_ref()
                        .map(|d| format!("{} [{}]", d.name, d.occurrence + 1))
                        .unwrap_or(if route.use_default_input {
                            "OS default input".into()
                        } else {
                            "Input disabled".into()
                        }),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut selection,
                        (false, None),
                        "Input disabled (output recording only)",
                    );
                    ui.selectable_value(&mut selection, (true, None), "OS default input");
                    for d in &catalog.inputs {
                        ui.selectable_value(
                            &mut selection,
                            (false, Some(d.selector.clone())),
                            format!("{} [{}]", d.selector.name, d.selector.occurrence + 1),
                        );
                    }
                });
            route.use_default_input = selection.0;
            route.input = selection.1;
            ui.horizontal_wrapped(|ui| {
                ui.label("Input map L/R");
                for v in &mut route.input_channels {
                    ui.add(egui::DragValue::new(v).range(0..=63));
                }
                ui.label("Output map L/R");
                for v in &mut route.output_channels {
                    ui.add(egui::DragValue::new(v).range(0..=63));
                }
            });
            ui.label("Unified DSP is stereo. Hardware channels use the maps above. Monitoring starts muted after every start/reconnect.");
        }
        if let Some(report) = &status.last_recording {
            ui.label(format!(
                "Last take: {} captured / {} written / {} dropped frames; {} write errors",
                report.captured_frames,
                report.written_frames,
                report.dropped_frames,
                report.write_errors
            ));
            if let Some(e) = &report.error {
                ui.colored_label(egui::Color32::YELLOW, e);
            }
        }
        ui.checkbox(
            &mut draft.reconnect,
            "Retry after device loss or stream errors",
        );
        ui.checkbox(
            &mut draft.fallback_to_default,
            "Allow fallback to OS default devices",
        );
        ui.add(
            egui::DragValue::new(&mut draft.retry_ms)
                .range(250..=30000)
                .suffix(" ms between checks"),
        );
        ui.horizontal_wrapped(|ui| {
            for (label, a) in [
                ("Refresh devices", SettingsAction::Refresh),
                ("Apply", SettingsAction::Apply),
                ("Save preferences", SettingsAction::Save),
                ("Load preferences", SettingsAction::Load),
                ("Play / retry", SettingsAction::Retry),
                ("Stop", SettingsAction::Stop),
            ] {
                if ui.button(label).clicked() {
                    action = Some(a);
                }
            }
        });
        egui::CollapsingHeader::new("Supported output configurations").show(ui, |ui| {
            for d in &catalog.outputs {
                ui.label(format!(
                    "{} [{}]",
                    d.selector.name,
                    d.selector.occurrence + 1
                ));
                for r in &d.configs {
                    ui.label(format!(
                        "{} ch · {}–{} Hz · {:?} · buffer {:?}",
                        r.channels, r.min_rate, r.max_rate, r.format, r.buffer_range
                    ));
                }
            }
            for w in &catalog.warnings {
                ui.label(w);
            }
        });
        action
    }
}
