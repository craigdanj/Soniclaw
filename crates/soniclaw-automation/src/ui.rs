//! Reusable egui lane editor. Work on a draft; validate and publish off the callback.
use super::*;
use egui::{Color32, Pos2, Sense, Stroke};
pub struct Editor {
    pub selected: usize,
    pub grid: f64,
    pub span: f64,
    pub cursor: f64,
    value: f32,
    recorder: Recorder,
    recording: bool,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            selected: 0,
            grid: 0.25,
            span: 8.0,
            cursor: 0.0,
            value: 0.5,
            recorder: Recorder::default(),
            recording: false,
        }
    }
}
impl Editor {
    /// Clear a held gesture/latch when the embedding transport stops.
    pub fn cancel_gesture(&mut self) {
        self.recorder.stop();
    }

    /// `cursor` is supplied by the embedding application, in beats.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        parameters: &[String],
    ) -> Result<(), Error> {
        ui.horizontal(|ui| {
            ui.label("Grid (beats)");
            ui.add(
                egui::DragValue::new(&mut self.grid)
                    .range(0.0..=4.0)
                    .speed(0.01),
            );
            ui.label("View");
            ui.add(egui::DragValue::new(&mut self.span).range(1.0..=128.0));
            if ui.button("Add lane").clicked() {
                if let Some(id) = parameters
                    .iter()
                    .find(|p| !doc.lanes.iter().any(|l| &l.parameter == *p))
                {
                    if doc.lanes.len() < MAX_LANES {
                        doc.lanes.push(Lane {
                            parameter: id.clone(),
                            enabled: true,
                            mode: Mode::Read,
                            points: vec![Point {
                                beat: 0.0,
                                value: 0.5,
                                curve: Curve::Linear,
                            }],
                        });
                        self.selected = doc.lanes.len() - 1;
                    }
                }
            }
            let mut looping = doc.loop_range.is_some();
            if ui.checkbox(&mut looping, "Loop view").changed() {
                doc.loop_range = looping.then_some([0.0, self.span]);
            }
        });
        if doc.lanes.is_empty() {
            ui.label("Add a lane to automate a rack parameter.");
            return Ok(());
        }
        self.selected = self.selected.min(doc.lanes.len() - 1);
        egui::ComboBox::from_id_salt("automation-lane")
            .selected_text(&doc.lanes[self.selected].parameter)
            .show_ui(ui, |ui| {
                for (i, l) in doc.lanes.iter().enumerate() {
                    ui.selectable_value(&mut self.selected, i, &l.parameter);
                }
            });
        let lane = &mut doc.lanes[self.selected];
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("automation-target")
                .selected_text(&lane.parameter)
                .show_ui(ui, |ui| {
                    for id in parameters {
                        ui.selectable_value(&mut lane.parameter, id.clone(), id);
                    }
                });
            ui.checkbox(&mut lane.enabled, "Enabled");
            for (m, name) in [
                (Mode::Read, "Read"),
                (Mode::Touch, "Touch"),
                (Mode::Latch, "Latch"),
            ] {
                if ui.selectable_value(&mut lane.mode, m, name).changed() {
                    self.recorder.stop();
                }
            }
            ui.checkbox(&mut self.recording, "Record draft");
        });
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 170.0),
            Sense::click_and_drag(),
        );
        let pos = |b: f64, v: f32| {
            Pos2::new(
                rect.left() + (b / self.span) as f32 * rect.width(),
                rect.bottom() - v * rect.height(),
            )
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_gray(22));
        for i in 0..=16 {
            let x = rect.left() + i as f32 / 16.0 * rect.width();
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_gray(40)),
            );
        }
        let path: Vec<_> = (0..=256)
            .map(|i| {
                let beat = self.span * i as f64 / 256.0;
                pos(beat, lane.value_at(beat).unwrap_or(0.0))
            })
            .collect();
        painter.add(egui::Shape::line(
            path,
            Stroke::new(2.0, Color32::from_rgb(84, 222, 190)),
        ));
        for p in &lane.points {
            painter.circle_filled(pos(p.beat, p.value), 4.0, Color32::WHITE);
        }
        let x = pos(self.cursor, 0.0).x;
        if rect.x_range().contains(x) {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.0, Color32::YELLOW),
            );
        }
        if response.double_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let b = ((p.x - rect.left()) / rect.width()) as f64 * self.span;
                let v = ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0);
                doc.write(self.selected, b, v, self.grid)?;
            }
        }
        ui.label(
            "Double-click the graph to add points. Edit exact positions and outgoing curves below.",
        );
        let lane = &mut doc.lanes[self.selected];
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(110.0)
            .show(ui, |ui| {
                for (i, p) in lane.points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut p.beat)
                                .range(0.0..=1_000_000.0)
                                .speed(self.grid.max(0.01)),
                        );
                        ui.add(egui::Slider::new(&mut p.value, 0.0..=1.0));
                        egui::ComboBox::from_id_salt(("curve", i))
                            .selected_text(format!("{:?}", p.curve))
                            .show_ui(ui, |ui| {
                                for c in [
                                    Curve::Step,
                                    Curve::Linear,
                                    Curve::Smooth,
                                    Curve::Bezier { c1: 0.1, c2: 0.9 },
                                ] {
                                    ui.selectable_value(&mut p.curve, c, format!("{c:?}"));
                                }
                            });
                        if let Curve::Bezier { c1, c2 } = &mut p.curve {
                            ui.add(egui::DragValue::new(c1).range(0.0..=1.0).speed(0.01));
                            ui.add(egui::DragValue::new(c2).range(0.0..=1.0).speed(0.01));
                        }
                        if ui.button("×").clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
        if let Some(i) = remove {
            lane.points.remove(i);
        }
        lane.points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        if self.recording {
            let r = ui.add(egui::Slider::new(&mut self.value, 0.0..=1.0).text("Record value"));
            if r.dragged() || r.changed() {
                self.recorder.begin(self.value)?;
            }
            if r.drag_stopped() {
                self.recorder.end();
            }
            self.recorder
                .tick(doc, self.selected, self.cursor, self.grid)?;
            if ui.button("Stop gesture / clear latch").clicked() {
                self.recorder.stop();
            }
        } else {
            self.recorder.stop();
        }
        if ui.button("Delete lane").clicked() {
            doc.lanes.remove(self.selected);
            self.recorder.stop();
        }
        doc.validate()
    }
}
