//! Control-side draft editors. Validate/build before publication to audio.
use egui::*;
use soniclaw_state::{Connection, ProjectState, SampleZone};

#[derive(Default)]
pub struct GraphEditor {
    pub selected: usize,
    from: Option<String>,
}
impl GraphEditor {
    pub fn show(&mut self, ui: &mut Ui, p: &mut ProjectState) -> bool {
        let mut changed = false;
        ui.label(
            "Drag nodes. Click OUT, then IN to connect. Apply validates and publishes the draft.",
        );
        let cols = ((ui.available_width() - 30.0) / 190.0).floor().max(1.0) as usize;
        let height = ((p.nodes.len().div_ceil(cols)) * 105 + 35).max(320) as f32;
        let (canvas, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 8.0, crate::toolkit::surface(ui));
        let width = 145.0;
        let mut rects = vec![];
        for (i, n) in p.nodes.iter().enumerate() {
            let xy = p.layout.get(&n.id).copied().unwrap_or([
                20.0 + (i % cols) as f32 * 190.0,
                25.0 + (i / cols) as f32 * 105.0,
            ]);
            rects.push(Rect::from_min_size(
                canvas.min
                    + vec2(
                        xy[0].clamp(10.0, (canvas.width() - width - 10.0).max(10.0)),
                        xy[1].clamp(10.0, canvas.height() - 75.0),
                    ),
                vec2(width, 65.0),
            ));
        }
        for edge in &p.connections {
            if let (Some(a), Some(b)) = (
                p.nodes.iter().position(|n| n.id == edge.from),
                p.nodes.iter().position(|n| n.id == edge.to),
            ) {
                let a = pos2(rects[a].right() + 7.0, rects[a].center().y);
                let b = pos2(rects[b].left() - 7.0, rects[b].center().y);
                let bend = (a.x - b.x).abs().max(50.0) * 0.5;
                painter.add(epaint::CubicBezierShape::from_points_stroke(
                    [a, a + vec2(bend, 0.0), b - vec2(bend, 0.0), b],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.0, crate::toolkit::accent(ui)),
                ));
            }
        }
        let mut connect = None;
        for (i, n) in p.nodes.iter().enumerate() {
            let r = rects[i];
            let hit = ui.interact(r, ui.id().with(("node", &n.id)), Sense::click_and_drag());
            if hit.clicked() {
                self.selected = i;
            }
            if hit.dragged() {
                let delta = hit.drag_motion();
                p.layout.insert(
                    n.id.clone(),
                    [
                        (r.left() - canvas.left() + delta.x)
                            .clamp(10.0, (canvas.width() - width - 10.0).max(10.0)),
                        (r.top() - canvas.top() + delta.y).clamp(10.0, canvas.height() - 75.0),
                    ],
                );
                changed = true;
            }
            painter.rect_filled(
                r,
                7.0,
                if i == self.selected {
                    Color32::from_rgb(27, 71, 69)
                } else {
                    Color32::from_rgb(25, 35, 49)
                },
            );
            painter.text(
                r.left_top() + vec2(12.0, 12.0),
                Align2::LEFT_TOP,
                &n.id,
                FontId::proportional(16.0),
                Color32::WHITE,
            );
            painter.text(
                r.left_bottom() + vec2(12.0, -12.0),
                Align2::LEFT_BOTTOM,
                format!("{:?}", n.kind),
                FontId::proportional(11.0),
                Color32::GRAY,
            );
            for input in [true, false] {
                let at = pos2(
                    if input {
                        r.left() - 7.0
                    } else {
                        r.right() + 7.0
                    },
                    r.center().y,
                );
                let h = ui.interact(
                    Rect::from_center_size(at, vec2(22.0, 24.0)),
                    ui.id().with((&n.id, input)),
                    Sense::click(),
                );
                painter.circle_filled(
                    at,
                    6.0,
                    if !input && self.from.as_ref() == Some(&n.id) {
                        Color32::YELLOW
                    } else {
                        crate::toolkit::accent(ui)
                    },
                );
                if h.clicked() {
                    if input {
                        if let Some(from) = self.from.take() {
                            connect = Some((from, n.id.clone()));
                        }
                    } else {
                        self.from = Some(n.id.clone());
                    }
                }
                h.on_hover_text(if input { "IN" } else { "OUT" });
            }
        }
        if let Some((from, to)) = connect {
            if from != to && !p.connections.iter().any(|c| c.from == from && c.to == to) {
                p.connections.push(Connection {
                    from,
                    to,
                    gain: 1.0,
                });
                changed = true;
            }
        }
        if let Some(n) = p.nodes.get_mut(self.selected) {
            ui.horizontal(|ui| {
                ui.label(format!("Selected: {}", n.id));
                changed |= ui
                    .add(Slider::new(&mut n.input_gain, 0.0..=2.0).text("External input"))
                    .changed();
                let mut output = p.outputs.contains_key(&n.id);
                if ui.checkbox(&mut output, "Main output").changed() {
                    changed = true;
                    if output {
                        p.outputs.insert(n.id.clone(), 1.0);
                    } else {
                        p.outputs.remove(&n.id);
                    }
                }
            });
        }
        let mut remove = None;
        ScrollArea::vertical()
            .id_salt("graph-edges")
            .max_height(100.0)
            .show(ui, |ui| {
                for (i, c) in p.connections.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} → {}", c.from, c.to));
                        changed |= ui
                            .add(Slider::new(&mut c.gain, -2.0..=2.0).text("Send"))
                            .changed();
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
        if let Some(i) = remove {
            p.connections.remove(i);
            changed = true;
        }
        changed
    }
}
#[derive(Default)]
pub struct SampleMapEditor {
    pub selected: usize,
    pub asset: usize,
    slices: usize,
    drag: Option<(usize, [u8; 2], [u8; 2])>,
}
impl SampleMapEditor {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        p: &mut ProjectState,
        waveform: &[f32],
        frames: usize,
    ) -> bool {
        let mut changed = false;
        self.asset = self.asset.min(p.assets.len().saturating_sub(1));
        let previous = self.asset;
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("zone-asset")
                .selected_text(
                    p.assets
                        .get(self.asset)
                        .map_or("No assets", |a| a.id.as_str()),
                )
                .show_ui(ui, |ui| {
                    for (i, a) in p.assets.iter().enumerate() {
                        ui.selectable_value(&mut self.asset, i, &a.id);
                    }
                });
            if ui.button("Add zone").clicked() {
                if let Some(a) = p.assets.get(self.asset) {
                    if p.zones.len() < 1024 {
                        p.zones.push(SampleZone::new(&a.id));
                        self.selected = p.zones.len() - 1;
                        changed = true;
                    }
                }
            }
            if ui.button("Delete zone").clicked() && self.selected < p.zones.len() {
                p.zones.remove(self.selected);
                self.selected = self.selected.saturating_sub(1);
                changed = true;
            }
        });
        if self.asset != previous {
            self.selected = p
                .assets
                .get(self.asset)
                .and_then(|a| p.zones.iter().position(|z| z.asset == a.id))
                .unwrap_or(usize::MAX);
            return changed;
        }
        ui.label("Key / velocity map · drag a zone to move its ranges.");
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
        let paint = ui.painter_at(rect);
        paint.rect_filled(rect, 6.0, crate::toolkit::surface(ui));
        for key in (0..=128).step_by(12) {
            let x = rect.left() + rect.width() * key as f32 / 128.0;
            paint.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, Color32::from_gray(45)),
            );
            paint.text(
                pos2(x + 3.0, rect.bottom() - 2.0),
                Align2::LEFT_BOTTOM,
                key.to_string(),
                FontId::proportional(10.0),
                Color32::GRAY,
            );
        }
        for (i, z) in p.zones.iter_mut().enumerate() {
            let r = Rect::from_min_max(
                pos2(
                    rect.left() + rect.width() * z.keys[0] as f32 / 128.0,
                    rect.bottom() - rect.height() * z.velocities[1] as f32 / 127.0,
                ),
                pos2(
                    rect.left() + rect.width() * (z.keys[1] as f32 + 1.0) / 128.0,
                    rect.bottom() - rect.height() * (z.velocities[0] as f32 - 1.0) / 127.0,
                ),
            );
            let h = ui.interact(r, ui.id().with(("zone", i)), Sense::click_and_drag());
            if h.clicked() || h.drag_started() {
                self.selected = i;
                self.asset = p.assets.iter().position(|a| a.id == z.asset).unwrap_or(0);
            }
            if h.drag_started() {
                self.drag = Some((i, z.keys, z.velocities));
            }
            if h.dragged() {
                if let Some((index, keys, vel)) = self.drag {
                    if index == i {
                        let dx = (h.drag_delta().x / rect.width() * 128.0).round() as i16;
                        let dy = (-h.drag_delta().y / rect.height() * 127.0).round() as i16;
                        let dx = dx.clamp(-(keys[0] as i16), 127 - keys[1] as i16);
                        let dy = dy.clamp(1 - vel[0] as i16, 127 - vel[1] as i16);
                        z.keys = [(keys[0] as i16 + dx) as u8, (keys[1] as i16 + dx) as u8];
                        z.velocities = [(vel[0] as i16 + dy) as u8, (vel[1] as i16 + dy) as u8];
                        changed = true;
                    }
                }
            }
            if h.drag_stopped() {
                self.drag = None;
            }
            paint.rect_filled(
                r.shrink(1.0),
                3.0,
                if i == self.selected {
                    Color32::from_rgb(27, 116, 103)
                } else {
                    Color32::from_rgb(40, 68, 90)
                },
            );
            paint.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("{} · RR {}", z.asset, z.group),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        if self.asset != previous {
            return changed;
        }
        if let Some(z) = p.zones.get_mut(self.selected) {
            ui.horizontal_wrapped(|ui| {
                ui.label("Keys");
                changed |= ui
                    .add(DragValue::new(&mut z.keys[0]).range(0..=127))
                    .changed();
                let low = z.keys[0];
                changed |= ui
                    .add(DragValue::new(&mut z.keys[1]).range(low..=127))
                    .changed();
                ui.label("Velocity");
                changed |= ui
                    .add(DragValue::new(&mut z.velocities[0]).range(1..=127))
                    .changed();
                let low = z.velocities[0];
                changed |= ui
                    .add(DragValue::new(&mut z.velocities[1]).range(low..=127))
                    .changed();
                ui.label("RR group");
                changed |= ui
                    .add(DragValue::new(&mut z.group).range(0..=255))
                    .changed();
                changed |= ui
                    .add(Slider::new(&mut z.gain, 0.0..=4.0).text("Gain"))
                    .changed();
                if ui.checkbox(&mut z.one_shot, "One shot").changed() {
                    changed = true;
                    if z.one_shot {
                        z.loop_frames = None;
                        z.loop_crossfade = 0;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .checkbox(&mut z.release_trigger, "Release trigger")
                    .changed()
                {
                    changed = true;
                    if z.release_trigger {
                        z.one_shot = true;
                        z.loop_frames = None;
                        z.loop_crossfade = 0;
                    }
                }
                for (label, fade) in [
                    ("Velocity fade in", &mut z.velocity_fade_in),
                    ("Velocity fade out", &mut z.velocity_fade_out),
                ] {
                    let mut enabled = fade.is_some();
                    if ui.checkbox(&mut enabled, label).changed() {
                        *fade = if enabled { Some([1, 127]) } else { None };
                        changed = true;
                    }
                    if let Some([a, b]) = fade {
                        changed |= ui.add(DragValue::new(a).range(0..=126)).changed();
                        let lo = *a + 1;
                        changed |= ui.add(DragValue::new(b).range(lo..=127)).changed();
                    }
                }
                changed |= ui
                    .checkbox(&mut z.velocity_power, "Equal-power velocity fades")
                    .changed();
            });
            ui.label("Different RR groups layer; the same RR group selects alternatives.");
            if frames > 0 {
                z.start_frame = z.start_frame.min(frames - 1);
                let mut end = z
                    .end_frame
                    .unwrap_or(frames)
                    .clamp(z.start_frame + 1, frames);
                if z.end_frame.is_some() {
                    z.end_frame = Some(end);
                }
                ui.horizontal(|ui| {
                    ui.label("Region start/end");
                    changed |= ui
                        .add(DragValue::new(&mut z.start_frame).range(0..=end - 1))
                        .changed();
                    if ui
                        .add(DragValue::new(&mut end).range(z.start_frame + 1..=frames))
                        .changed()
                    {
                        z.end_frame = Some(end);
                        changed = true;
                    }
                    let mut looping = z.loop_frames.is_some();
                    if ui
                        .add_enabled(!z.one_shot, Checkbox::new(&mut looping, "Loop"))
                        .changed()
                    {
                        z.loop_frames = if looping {
                            Some([z.start_frame, end])
                        } else {
                            None
                        };
                        z.loop_crossfade = 0;
                        changed = true;
                    }
                });
                if let Some([a, b]) = &mut z.loop_frames {
                    *a = (*a).clamp(z.start_frame, end - 1);
                    *b = (*b).clamp(*a + 1, end);
                    z.loop_crossfade = z.loop_crossfade.min((*b - *a) / 2);
                }
                if let Some([mut a, mut b]) = z.loop_frames {
                    ui.horizontal(|ui| {
                        ui.label("Loop start/end");
                        changed |= ui
                            .add(DragValue::new(&mut a).range(z.start_frame..=b - 1))
                            .changed();
                        changed |= ui.add(DragValue::new(&mut b).range(a + 1..=end)).changed();
                        changed |= ui
                            .add(
                                DragValue::new(&mut z.loop_crossfade)
                                    .range(0..=((b - a) / 2).min(65536)),
                            )
                            .changed();
                        ui.label("crossfade frames");
                    });
                    z.loop_frames = Some([a, b]);
                }
                let (wave, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 100.0), Sense::hover());
                let painter = ui.painter_at(wave);
                painter.rect_filled(wave, 5.0, crate::toolkit::surface(ui));
                let points: Vec<_> = waveform
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        pos2(
                            wave.left() + wave.width() * i as f32 / waveform.len().max(1) as f32,
                            wave.center().y - v.clamp(-1.0, 1.0) * wave.height() * 0.45,
                        )
                    })
                    .collect();
                if points.len() > 1 {
                    painter.add(Shape::line(
                        points,
                        Stroke::new(1.0, crate::toolkit::accent(ui)),
                    ));
                }
                let mut markers =
                    vec![(0, z.start_frame, Color32::WHITE), (1, end, Color32::WHITE)];
                if let Some([a, b]) = z.loop_frames {
                    markers.extend([(2, a, Color32::YELLOW), (3, b, Color32::YELLOW)]);
                }
                for (kind, frame, color) in markers {
                    let x = wave.left() + wave.width() * frame as f32 / frames as f32;
                    painter.line_segment(
                        [pos2(x, wave.top()), pos2(x, wave.bottom())],
                        Stroke::new(2.0, color),
                    );
                    let h = ui.interact(
                        Rect::from_min_max(pos2(x - 5.0, wave.top()), pos2(x + 5.0, wave.bottom())),
                        ui.id().with(("marker", kind)),
                        Sense::drag(),
                    );
                    if h.dragged() {
                        if let Some(at) = h.interact_pointer_pos() {
                            let n = (((at.x - wave.left()) / wave.width()).clamp(0.0, 1.0)
                                * frames as f32)
                                .round() as usize;
                            match kind {
                                0 => z.start_frame = n.min(end - 1),
                                1 => z.end_frame = Some(n.clamp(z.start_frame + 1, frames)),
                                2 => {
                                    if let Some(l) = &mut z.loop_frames {
                                        l[0] = n.clamp(z.start_frame, l[1] - 1)
                                    }
                                }
                                3 => {
                                    if let Some(l) = &mut z.loop_frames {
                                        l[1] = n.clamp(l[0] + 1, end)
                                    }
                                }
                                _ => {}
                            }
                            changed = true;
                        }
                    }
                }
            }
        }
        if let Some(a) = p.assets.get_mut(self.asset) {
            ui.horizontal(|ui| {
                ui.label("Asset root note");
                changed |= ui
                    .add(DragValue::new(&mut a.root_note).range(0..=127))
                    .changed();
                if self.slices == 0 {
                    self.slices = 4;
                }
                ui.add(DragValue::new(&mut self.slices).range(2..=16));
                if ui.button("Equal slices from key 36").clicked()
                    && frames >= self.slices
                    && p.zones.len() + self.slices <= 1024
                {
                    for i in 0..self.slices {
                        let mut z = SampleZone::new(&a.id);
                        z.keys = [36 + i as u8; 2];
                        z.start_frame = i * frames / self.slices;
                        z.end_frame = Some((i + 1) * frames / self.slices);
                        p.zones.push(z);
                    }
                    changed = true;
                }
            });
        }
        changed
    }
}
