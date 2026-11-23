//! Bounded waveform envelope and source/beat warp editing widget. File I/O stays with the caller.
use egui::*;
use soniclaw_studio::tempo_cache::{Marker, Mode, Recipe};
use soniclaw_waveform::{Peak, Thumbnail};
pub struct Editor {
    pub view: [f64; 2],
    pub selected: Option<usize>,
    pub error: String,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            view: [0., 1.],
            selected: None,
            error: String::new(),
        }
    }
}
impl Editor {
    /// Returns true when the recipe changed. Caller validates/applies the isolated draft.
    pub fn show(&mut self, ui: &mut Ui, thumbnail: &Thumbnail, recipe: &mut Recipe) -> bool {
        let before = recipe.clone();
        ui.horizontal(|ui| {
            ui.label("Source view");
            ui.add(Slider::new(&mut self.view[0], 0.0..=0.99).text("from"));
            ui.add(Slider::new(&mut self.view[1], 0.01..=1.0).text("to"));
            if ui.button("Full source").clicked() {
                self.view = [0., 1.];
            }
        });
        self.view[1] = self.view[1].max(self.view[0] + 0.001).min(1.);
        let frames = thumbnail.frames();
        let a = (frames as f64 * self.view[0]) as u64;
        let b = (frames as f64 * self.view[1]) as u64;
        let width = ui.available_width().clamp(64., 2048.);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 140.), Sense::click_and_drag());
        let mut peaks = vec![Peak::default(); width as usize];
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4., Color32::from_gray(22));
        if thumbnail.query(0, a, b, &mut peaks).is_ok() {
            for (i, p) in peaks.iter().enumerate() {
                let x = rect.left() + i as f32;
                painter.line_segment(
                    [
                        pos2(x, rect.center().y - p.min * 60.),
                        pos2(x, rect.center().y - p.max * 60.),
                    ],
                    Stroke::new(1., Color32::from_rgb(80, 200, 180)),
                );
            }
        }
        let x_for = |f: u64| {
            rect.left() + ((f as f64 - a as f64) / (b - a).max(1) as f64) as f32 * rect.width()
        };
        for (i, m) in recipe.markers.iter().enumerate() {
            let x = x_for(m.source_frame);
            if rect.x_range().contains(x) {
                painter.line_segment(
                    [pos2(x, rect.top()), pos2(x, rect.bottom())],
                    Stroke::new(
                        2.,
                        if self.selected == Some(i) {
                            Color32::YELLOW
                        } else {
                            Color32::LIGHT_BLUE
                        },
                    ),
                );
                painter.text(
                    pos2(x + 3., rect.top() + 3.),
                    Align2::LEFT_TOP,
                    format!("{:.2} beat", m.beat),
                    FontId::monospace(11.),
                    Color32::WHITE,
                );
            }
        }
        if response.drag_started() || response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.selected = recipe
                    .markers
                    .iter()
                    .enumerate()
                    .min_by(|(_, l), (_, r)| {
                        (x_for(l.source_frame) - pos.x)
                            .abs()
                            .total_cmp(&(x_for(r.source_frame) - pos.x).abs())
                    })
                    .filter(|(_, m)| (x_for(m.source_frame) - pos.x).abs() < 18.)
                    .map(|(i, _)| i);
            }
        }
        if response.dragged() {
            if let (Some(i), Some(pos)) = (self.selected, response.interact_pointer_pos()) {
                let f = (a as f64
                    + ((pos.x - rect.left()) / rect.width()).clamp(0., 1.) as f64 * (b - a) as f64)
                    .round() as u64;
                let lo = if i > 0 {
                    recipe.markers[i - 1].source_frame + 1
                } else {
                    0
                };
                let hi = recipe
                    .markers
                    .get(i + 1)
                    .map_or(frames, |m| m.source_frame.saturating_sub(1));
                recipe.markers[i].source_frame = f.clamp(lo, hi.max(lo));
            }
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut recipe.mode, Mode::PreservePitch, "Preserve pitch");
            ui.selectable_value(&mut recipe.mode, Mode::Repitch, "Repitch");
            if ui
                .add_enabled(recipe.markers.len() < 256, Button::new("Insert midpoint"))
                .clicked()
            {
                if let Some((i, p)) = recipe
                    .markers
                    .windows(2)
                    .enumerate()
                    .max_by_key(|(_, p)| p[1].source_frame - p[0].source_frame)
                {
                    let m = Marker {
                        source_frame: (p[0].source_frame + p[1].source_frame) / 2,
                        beat: (p[0].beat + p[1].beat) * 0.5,
                    };
                    recipe.markers.insert(i + 1, m);
                    self.selected = Some(i + 1);
                }
            }
        });
        if let Some(i) = self.selected.filter(|&i| i < recipe.markers.len()) {
            ui.horizontal(|ui| {
                ui.label(format!("Marker {} source frame", i + 1));
                ui.add(DragValue::new(&mut recipe.markers[i].source_frame).range(0..=frames));
                ui.label("Target beat");
                ui.add(
                    DragValue::new(&mut recipe.markers[i].beat)
                        .range(0.0..=1_000_000.)
                        .speed(0.01),
                );
                if ui
                    .add_enabled(recipe.markers.len() > 2, Button::new("Remove marker"))
                    .clicked()
                {
                    recipe.markers.remove(i);
                    self.selected = None;
                }
            });
        }
        serde_json::to_string(&before).ok() != serde_json::to_string(recipe).ok()
    }
}
