use egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use soniclaw_core::{MidiMessage, Parameter};
use soniclaw_dsp::AdsrSettings;
const ACCENT: Color32 = Color32::from_rgb(67, 216, 172);
fn position(parameter: &Parameter, logarithmic: bool) -> f32 {
    let i = parameter.info();
    if logarithmic && i.min > 0.0 {
        (parameter.get() / i.min).ln() / (i.max / i.min).ln()
    } else {
        parameter.normalized()
    }
}
fn set_position(parameter: &Parameter, t: f32, logarithmic: bool) {
    let i = parameter.info();
    let t = t.clamp(0.0, 1.0);
    let _ = parameter.set(if logarithmic && i.min > 0.0 {
        i.min * (i.max / i.min).powf(t)
    } else {
        i.min + t * (i.max - i.min)
    });
}
pub fn parameter_knob(
    ui: &mut egui::Ui,
    parameter: &Parameter,
    logarithmic: bool,
) -> egui::Response {
    ui.vertical(|ui| {
        let info = parameter.info();
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::splat(66.0), Sense::click_and_drag());
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta().y);
            if delta != 0.0 {
                set_position(
                    parameter,
                    position(parameter, logarithmic) - delta / 150.0,
                    logarithmic,
                );
                response.mark_changed();
            }
        }
        if response.double_clicked() {
            let _ = parameter.set(info.default);
            response.mark_changed();
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.has_focus() {
            let step = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowRight) {
                    0.01
                } else if i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowLeft)
                {
                    -0.01
                } else {
                    0.0
                }
            });
            if step != 0.0 {
                set_position(
                    parameter,
                    position(parameter, logarithmic) + step,
                    logarithmic,
                );
                response.mark_changed();
            }
        }
        let center = rect.center();
        let radius = 24.0;
        ui.painter()
            .circle_filled(center, radius, Color32::from_rgb(43, 53, 68));
        let points: Vec<_> = (0..=48)
            .map(|n| {
                let angle = std::f32::consts::PI
                    * (0.75 + 1.5 * position(parameter, logarithmic) * n as f32 / 48.0);
                center + Vec2::angled(angle) * radius
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(3.0_f32, ACCENT)));
        let angle = std::f32::consts::PI * (0.75 + 1.5 * position(parameter, logarithmic));
        ui.painter().line_segment(
            [
                center + Vec2::angled(angle) * 12.0,
                center + Vec2::angled(angle) * 20.0,
            ],
            Stroke::new(3.0_f32, Color32::WHITE),
        );
        ui.label(info.name);
        ui.small(format!("{:.2}{}", parameter.get(), info.unit));
        response.on_hover_text("Drag vertically · double-click to reset · arrow keys to adjust")
    })
    .inner
}
pub fn xy_pad(ui: &mut egui::Ui, x: &Parameter, y: &Parameter) -> egui::Response {
    let (rect, mut response) =
        ui.allocate_exact_size(Vec2::new(220.0, 150.0), Sense::click_and_drag());
    if response.is_pointer_button_down_on() {
        if let Some(p) = response.interact_pointer_pos() {
            let _ = x.set_normalized((p.x - rect.left()) / rect.width());
            let _ = y.set_normalized(1.0 - (p.y - rect.top()) / rect.height());
            response.mark_changed();
        }
    }
    ui.painter()
        .rect_filled(rect, 6.0, Color32::from_rgb(27, 35, 47));
    for n in 1..4 {
        let t = n as f32 / 4.0;
        ui.painter().line_segment(
            [
                Pos2::new(rect.left() + rect.width() * t, rect.top()),
                Pos2::new(rect.left() + rect.width() * t, rect.bottom()),
            ],
            Stroke::new(1.0_f32, Color32::from_gray(50)),
        );
    }
    let p = Pos2::new(
        rect.left() + x.normalized() * rect.width(),
        rect.bottom() - y.normalized() * rect.height(),
    );
    ui.painter().circle_filled(p, 6.0, ACCENT);
    response.on_hover_text(format!("X: {} · Y: {}", x.info().name, y.info().name))
}
#[derive(Default)]
pub struct MeterBallistics {
    pub level: f32,
    pub peak_hold: f32,
    hold_seconds: f32,
}
impl MeterBallistics {
    pub fn update(&mut self, peak: f32, dt: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let peak = if peak.is_finite() { peak.max(0.0) } else { 0.0 };
        self.level = peak.max(self.level * (-dt / 0.18).exp());
        if peak >= self.peak_hold {
            self.peak_hold = peak;
            self.hold_seconds = 1.0;
        } else {
            self.hold_seconds = (self.hold_seconds - dt).max(0.0);
            if self.hold_seconds == 0.0 {
                self.peak_hold *= (-dt / 0.5).exp();
            }
        }
    }
    pub fn show(&self, ui: &mut egui::Ui) {
        super::level_meter(ui, self.level);
        let db = 20.0 * self.peak_hold.max(1e-6).log10();
        ui.small(format!(
            "Peak hold {db:.1} dBFS{}",
            if self.peak_hold >= 1.0 {
                " · CLIP"
            } else {
                ""
            }
        ));
    }
}
#[derive(Default)]
pub struct WaveformView {
    pub zoom: f32,
    pub offset: usize,
}
impl WaveformView {
    pub fn show(&mut self, ui: &mut egui::Ui, samples: &[f32], channels: usize) -> egui::Response {
        self.zoom = self.zoom.max(1.0);
        ui.add(
            egui::Slider::new(&mut self.zoom, 1.0..=64.0)
                .logarithmic(true)
                .text("Waveform zoom"),
        );
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(100.0), 120.0),
            Sense::click_and_drag(),
        );
        ui.painter()
            .rect_filled(rect, 5.0, Color32::from_rgb(22, 29, 40));
        if channels == 0 || samples.is_empty() {
            return response;
        }
        let total = samples.len() / channels;
        let visible = ((total as f32 / self.zoom) as usize).max(1).min(total);
        self.offset = self.offset.min(total - visible);
        if response.dragged() {
            let delta = ui.input(|i| i.pointer.delta().x) / rect.width() * visible as f32;
            self.offset =
                (self.offset as f32 - delta).clamp(0.0, (total - visible) as f32) as usize;
        }
        let columns = (rect.width() as usize).max(1);
        for column in 0..columns {
            let start = self.offset + column * visible / columns;
            let end = (self.offset
                + ((column + 1) * visible / columns).max(column * visible / columns + 1))
            .min(total);
            let (mut min, mut max) = (0.0_f32, 0.0_f32);
            for frame in start..end {
                for &value in &samples[frame * channels..(frame + 1) * channels] {
                    if value.is_finite() {
                        min = min.min(value);
                        max = max.max(value);
                    }
                }
            }
            let x = rect.left() + column as f32;
            let y = rect.center().y;
            ui.painter().line_segment(
                [
                    Pos2::new(x, y - max.clamp(-1.0, 1.0) * 52.0),
                    Pos2::new(x, y - min.clamp(-1.0, 1.0) * 52.0),
                ],
                Stroke::new(1.0_f32, ACCENT),
            );
        }
        response.on_hover_text("Drag horizontally to pan")
    }
}
pub fn envelope_editor(ui: &mut egui::Ui, settings: &mut AdsrSettings) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (name, value, range) in [
            ("Attack", &mut settings.attack, 0.001..=5.0),
            ("Decay", &mut settings.decay, 0.001..=5.0),
            ("Release", &mut settings.release, 0.001..=10.0),
        ] {
            changed |= ui
                .add(
                    egui::Slider::new(value, range)
                        .logarithmic(true)
                        .text(name)
                        .suffix(" s"),
                )
                .changed();
        }
    });
    changed |= ui
        .add(egui::Slider::new(&mut settings.sustain, 0.0..=1.0).text("Sustain"))
        .changed();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.0), Sense::hover());
    let total = (settings.attack + settings.decay + settings.release + 0.5).max(0.01);
    let point = |t: f32, v: f32| {
        Pos2::new(
            rect.left() + rect.width() * t / total,
            rect.bottom() - rect.height() * v,
        )
    };
    let points = vec![
        point(0.0, 0.0),
        point(settings.attack, 1.0),
        point(settings.attack + settings.decay, settings.sustain),
        point(settings.attack + settings.decay + 0.5, settings.sustain),
        point(total, 0.0),
    ];
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.0_f32, ACCENT)));
    changed
}
/// Magnitude response for the RBJ low-pass topology used by soniclaw-dsp.
pub fn lowpass_response_db(frequency: f32, cutoff: f32, q: f32, sample_rate: f32) -> Option<f32> {
    if [frequency, cutoff, q, sample_rate]
        .iter()
        .any(|v| !v.is_finite())
        || frequency < 0.0
        || cutoff <= 0.0
        || q <= 0.0
        || sample_rate <= 0.0
        || frequency >= sample_rate * 0.5
    {
        return None;
    }
    let w0 = std::f32::consts::TAU * cutoff.min(sample_rate * 0.49) / sample_rate;
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b0 = (1.0 - w0.cos()) * 0.5 / a0;
    let b1 = 2.0 * b0;
    let b2 = b0;
    let a1 = -2.0 * w0.cos() / a0;
    let a2 = (1.0 - alpha) / a0;
    let w = std::f32::consts::TAU * frequency / sample_rate;
    let nr = b0 + b1 * w.cos() + b2 * (2.0 * w).cos();
    let ni = -b1 * w.sin() - b2 * (2.0 * w).sin();
    let dr = 1.0 + a1 * w.cos() + a2 * (2.0 * w).cos();
    let di = -a1 * w.sin() - a2 * (2.0 * w).sin();
    Some(
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di).max(1e-20))
            .max(1e-12)
            .log10(),
    )
}
pub fn eq_editor(
    ui: &mut egui::Ui,
    cutoff: &Parameter,
    q: f32,
    sample_rate: f32,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 130.0),
        Sense::click_and_drag(),
    );
    ui.painter()
        .rect_filled(rect, 5.0, Color32::from_rgb(22, 29, 40));
    let upper = (sample_rate * 0.49).clamp(21.0, 20000.0);
    let hz = |x: f32| 20.0 * (upper / 20.0).powf(x);
    if response.is_pointer_button_down_on() {
        if let Some(p) = response.interact_pointer_pos() {
            let _ = cutoff.set(hz(((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0)));
            response.mark_changed();
        }
    }
    let points: Vec<_> = (0..256)
        .filter_map(|n| {
            let x = n as f32 / 255.0;
            lowpass_response_db(hz(x), cutoff.get(), q, sample_rate).map(|db| {
                Pos2::new(
                    rect.left() + x * rect.width(),
                    rect.top() + ((6.0 - db) / 66.0).clamp(0.0, 1.0) * rect.height(),
                )
            })
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(2.0_f32, ACCENT)));
    response.on_hover_text("Click or drag to set cutoff")
}
/// Mouse/touch piano with black keys, drag transitions and release on focus loss.
/// The caller owns held state and forwards returned messages to its MIDI bridge.
pub fn piano_keyboard(
    ui: &mut egui::Ui,
    base: u8,
    octaves: u8,
    held: &mut [bool; 128],
) -> Vec<MidiMessage> {
    let octaves = octaves.clamp(1, 4) as usize;
    let whites = octaves * 7;
    let width = ui.available_width().min(whites as f32 * 42.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 110.0), Sense::click_and_drag());
    let key_width = width / whites as f32;
    let notes = [0u8, 2, 4, 5, 7, 9, 11];
    let mut chosen = None;
    let pointer = response
        .interact_pointer_pos()
        .filter(|p| response.is_pointer_button_down_on() && rect.contains(*p));
    for octave in 0..octaves {
        for (i, &step) in notes.iter().enumerate() {
            let note = base as usize + octave * 12 + step as usize;
            if note >= 128 {
                continue;
            }
            let key = egui::Rect::from_min_size(
                rect.min + Vec2::new((octave * 7 + i) as f32 * key_width, 0.0),
                Vec2::new(key_width - 1.0, 110.0),
            );
            ui.painter().rect_filled(
                key,
                2.0,
                if held[note] {
                    ACCENT
                } else {
                    Color32::from_gray(220)
                },
            );
            if pointer.is_some_and(|p| key.contains(p)) {
                chosen = Some(note);
            }
        }
    }
    for octave in 0..octaves {
        for (white, step) in [(0, 1usize), (1, 3), (3, 6), (4, 8), (5, 10)] {
            let note = base as usize + octave * 12 + step;
            if note >= 128 {
                continue;
            }
            let key = egui::Rect::from_min_size(
                rect.min
                    + Vec2::new(
                        (octave * 7 + white + 1) as f32 * key_width - key_width * 0.30,
                        0.0,
                    ),
                Vec2::new(key_width * 0.60, 66.0),
            );
            ui.painter().rect_filled(
                key,
                2.0,
                if held[note] {
                    ACCENT
                } else {
                    Color32::from_rgb(28, 34, 43)
                },
            );
            if pointer.is_some_and(|p| key.contains(p)) {
                chosen = Some(note);
            }
        }
    }
    if !ui.ctx().input(|i| i.viewport().focused.unwrap_or(true)) {
        chosen = None;
    }
    let mut messages = Vec::new();
    for (note, state) in held.iter_mut().enumerate() {
        let next = chosen == Some(note);
        if next != *state {
            messages.push(if next {
                MidiMessage::NoteOn {
                    channel: 0,
                    note: note as u8,
                    velocity: 100,
                }
            } else {
                MidiMessage::NoteOff {
                    channel: 0,
                    note: note as u8,
                    velocity: 0,
                }
            });
            *state = next;
        }
    }
    messages
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::ParameterInfo;
    #[test]
    fn meter_hold_and_release_are_time_based() {
        let mut meter = MeterBallistics::default();
        meter.update(1.0, 0.016);
        meter.update(0.0, 0.5);
        assert_eq!(meter.peak_hold, 1.0);
        assert!(meter.level < 0.1);
        meter.update(0.0, 0.6);
        assert!(meter.peak_hold < 0.5);
    }
    #[test]
    fn response_matches_butterworth_cutoff() {
        let db =
            lowpass_response_db(1000.0, 1000.0, std::f32::consts::FRAC_1_SQRT_2, 48000.0).unwrap();
        assert!((db + 3.0103).abs() < 0.02);
        assert!(lowpass_response_db(20000.0, 1000.0, 0.707, 48000.0).unwrap() < -50.0);
    }
    #[test]
    fn controls_render_without_a_native_window() {
        let context = egui::Context::default();
        let p = Parameter::new(ParameterInfo {
            id: "p",
            name: "Control",
            min: 20.0,
            max: 20000.0,
            default: 1000.0,
            unit: "Hz",
        })
        .unwrap();
        let mut waveform = WaveformView::default();
        let mut adsr = AdsrSettings::default();
        let mut held = [false; 128];
        let mut frame = context.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                parameter_knob(ui, &p, true);
                xy_pad(ui, &p, &p);
                eq_editor(ui, &p, 0.707, 48000.0);
                waveform.show(ui, &[0.0, 1.0, -1.0], 1);
                envelope_editor(ui, &mut adsr);
                piano_keyboard(ui, 60, 1, &mut held);
            });
        });
        assert!(!frame.shapes.is_empty());
        frame.textures_delta.clear();
        assert_eq!(p.get(), 1000.0);
    }
}
