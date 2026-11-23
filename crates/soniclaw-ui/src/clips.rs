//! Draft arrangement and piano-roll views. The caller validates/rebuilds playback off the callback.
use egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use soniclaw_clips::{Document, Edit, Note};
#[derive(Default)]
pub struct ClipEditor {
    pub selected: Option<String>,
    pub seconds_per_screen: f64,
    pub snap_frames: u64,
    pub snap_beats: f64,
}
impl ClipEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
    ) -> Result<bool, soniclaw_core::Error> {
        document.validate()?;
        let map = soniclaw_transport_map(document)?;
        if !self.seconds_per_screen.is_finite() || self.seconds_per_screen <= 0. {
            self.seconds_per_screen = 10.;
        }
        ui.add(
            egui::Slider::new(&mut self.seconds_per_screen, 1.0..=120.0)
                .logarithmic(true)
                .text("Visible seconds"),
        );
        let width = ui.available_width().clamp(120., 8192.);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 270.), Sense::hover());
        let theme = crate::current_theme(ui.ctx());
        ui.painter_at(rect)
            .rect_filled(rect, 5., crate::theme_color(theme.surface));
        for lane in 0..6 {
            let y = rect.top() + lane as f32 * 42.;
            ui.painter_at(rect).line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1., crate::theme_color(theme.muted).gamma_multiply(0.3)),
            );
        }
        for second in 0..=self.seconds_per_screen.ceil() as usize {
            let x = rect.left() + second as f32 / self.seconds_per_screen as f32 * width;
            ui.painter_at(rect).line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1., crate::theme_color(theme.muted).gamma_multiply(0.2)),
            );
        }
        let mut edit = None;
        let to_x =
            |seconds: f64| rect.left() + seconds as f32 / self.seconds_per_screen as f32 * width;
        for clip in &document.audio {
            let x = to_x(clip.start_frame as f64 / document.sample_rate as f64);
            let end =
                to_x((clip.start_frame + clip.length_frames) as f64 / document.sample_rate as f64);
            let y = rect.top() + clip.track.min(5) as f32 * 42. + 5.;
            let box_rect = Rect::from_min_max(Pos2::new(x, y), Pos2::new(end.max(x + 8.), y + 32.));
            let response = ui.interact(
                box_rect.intersect(rect),
                ui.id().with(("audio-clip", &clip.id)),
                Sense::click_and_drag(),
            );
            if crate::accessibility::selectable(
                ui,
                &response,
                &format!("Clip {} on track {}", clip.id, clip.track + 1),
                self.selected.as_ref() == Some(&clip.id),
            ) {
                self.selected = Some(clip.id.clone());
                response.request_focus();
            }
            if response.drag_started() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(response.id, clip.start_frame));
            }
            if response.has_focus() && ui.is_enabled() {
                let step = self.snap_frames.max(1);
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    edit = Some(Edit::RemoveClip {
                        id: clip.id.clone(),
                    });
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    edit = Some(Edit::MoveAudio {
                        id: clip.id.clone(),
                        frame: clip.start_frame.saturating_add(step),
                        snap_frames: self.snap_frames,
                    });
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    edit = Some(Edit::MoveAudio {
                        id: clip.id.clone(),
                        frame: clip.start_frame.saturating_sub(step),
                        snap_frames: self.snap_frames,
                    });
                }
            }
            if response.dragged() {
                let anchor = ui
                    .ctx()
                    .data(|d| d.get_temp::<u64>(response.id))
                    .unwrap_or(clip.start_frame);
                let delta = response.total_drag_delta().unwrap_or_default().x as f64 / width as f64
                    * self.seconds_per_screen
                    * document.sample_rate as f64;
                edit = Some(Edit::MoveAudio {
                    id: clip.id.clone(),
                    frame: (anchor as f64 + delta).max(0.) as u64,
                    snap_frames: self.snap_frames,
                });
            }
            paint_clip(
                ui,
                box_rect.intersect(rect),
                &clip.id,
                self.selected.as_ref() == Some(&clip.id),
                &theme,
            );
        }
        for clip in &document.midi {
            let start = map.seconds_at(clip.start_beat)?;
            let end = map.seconds_at(clip.start_beat + clip.length_beats)?;
            let x = to_x(start);
            let y = rect.top() + clip.track.min(5) as f32 * 42. + 5.;
            let box_rect =
                Rect::from_min_max(Pos2::new(x, y), Pos2::new(to_x(end).max(x + 8.), y + 32.));
            let response = ui.interact(
                box_rect.intersect(rect),
                ui.id().with(("midi-clip", &clip.id)),
                Sense::click_and_drag(),
            );
            if crate::accessibility::selectable(
                ui,
                &response,
                &format!("Clip {} on track {}", clip.id, clip.track + 1),
                self.selected.as_ref() == Some(&clip.id),
            ) {
                self.selected = Some(clip.id.clone());
                response.request_focus();
            }
            if response.drag_started() {
                ui.ctx().data_mut(|d| d.insert_temp(response.id, start));
            }
            if response.has_focus() && ui.is_enabled() {
                let step = if self.snap_beats.is_finite() && self.snap_beats > 0. {
                    self.snap_beats
                } else {
                    0.25
                };
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    edit = Some(Edit::RemoveClip {
                        id: clip.id.clone(),
                    });
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    edit = Some(Edit::MoveMidi {
                        id: clip.id.clone(),
                        beat: clip.start_beat + step,
                        snap_beats: self.snap_beats,
                    });
                } else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    edit = Some(Edit::MoveMidi {
                        id: clip.id.clone(),
                        beat: (clip.start_beat - step).max(0.),
                        snap_beats: self.snap_beats,
                    });
                }
            }
            if response.dragged() {
                let anchor = ui
                    .ctx()
                    .data(|d| d.get_temp::<f64>(response.id))
                    .unwrap_or(start);
                let seconds = (anchor
                    + response.total_drag_delta().unwrap_or_default().x as f64 / width as f64
                        * self.seconds_per_screen)
                    .max(0.);
                edit = Some(Edit::MoveMidi {
                    id: clip.id.clone(),
                    beat: map.beat_at_seconds(seconds)?,
                    snap_beats: self.snap_beats,
                });
            }
            paint_clip(
                ui,
                box_rect.intersect(rect),
                &format!("{} · MIDI", clip.id),
                self.selected.as_ref() == Some(&clip.id),
                &theme,
            );
        }
        ui.collapsing("Accessible clip list", |ui| {
            for (id, track) in document
                .audio
                .iter()
                .map(|c| (&c.id, c.track))
                .chain(document.midi.iter().map(|c| (&c.id, c.track)))
            {
                if ui
                    .selectable_label(
                        self.selected.as_ref() == Some(id),
                        format!("{} · track {}", id, track + 1),
                    )
                    .clicked()
                {
                    self.selected = Some(id.clone());
                }
            }
        });
        let mut changed = false;
        if let Some(edit) = edit {
            document.apply(edit)?;
            changed = true;
        }
        if let Some(id) = self.selected.clone() {
            ui.label(&id);
            let mut action = None;
            if let Some(c) = document.audio.iter().find(|c| c.id == id) {
                let mut start = c.start_frame;
                if ui
                    .add(
                        egui::DragValue::new(&mut start)
                            .prefix("Start frame ")
                            .range(0..=1u64 << 48),
                    )
                    .changed()
                {
                    action = Some(Edit::MoveAudio {
                        id: id.clone(),
                        frame: start,
                        snap_frames: self.snap_frames,
                    });
                }
                let mut length = c.length_frames;
                let mut fade_in = c.fade_in;
                let mut fade_out = c.fade_out;
                ui.horizontal(|ui| {
                    ui.label("Length frames");
                    if ui
                        .add(egui::DragValue::new(&mut length).range(1..=u64::MAX))
                        .changed()
                    {
                        action = Some(Edit::ResizeAudio {
                            id: id.clone(),
                            length_frames: length,
                        });
                    }
                    ui.label("Fade in/out");
                    let a = ui.add(egui::DragValue::new(&mut fade_in).range(0..=length));
                    let b = ui.add(egui::DragValue::new(&mut fade_out).range(0..=length));
                    if a.changed() || b.changed() {
                        action = Some(Edit::FadeAudio {
                            id: id.clone(),
                            fade_in,
                            fade_out,
                        });
                    }
                });
            }
            if let Some(c) = document.midi.iter().find(|c| c.id == id) {
                let mut start = c.start_beat;
                if ui
                    .add(
                        egui::DragValue::new(&mut start)
                            .prefix("Start beat ")
                            .range(0.0..=1000000.),
                    )
                    .changed()
                {
                    action = Some(Edit::MoveMidi {
                        id: id.clone(),
                        beat: start,
                        snap_beats: self.snap_beats,
                    });
                }
                let mut length = c.length_beats;
                ui.horizontal(|ui| {
                    ui.label("Length beats");
                    if ui
                        .add(
                            egui::DragValue::new(&mut length)
                                .speed(0.25)
                                .range(0.001..=1000000.),
                        )
                        .changed()
                    {
                        action = Some(Edit::ResizeMidi {
                            id: id.clone(),
                            length_beats: length,
                        });
                    }
                });
            }
            if let Some(action) = action {
                document.apply(action)?;
                changed = true;
            }
            if ui.button("Remove selected").clicked() {
                document.apply(Edit::RemoveClip { id })?;
                self.selected = None;
                changed = true;
            }
        }
        Ok(changed)
    }
}
fn soniclaw_transport_map(
    document: &Document,
) -> Result<soniclaw_transport::TempoMap, soniclaw_core::Error> {
    soniclaw_transport::TempoMap::compile(&document.timeline)
}
fn paint_clip(ui: &egui::Ui, rect: Rect, text: &str, selected: bool, theme: &crate::Theme) {
    ui.painter_at(rect).rect_filled(
        rect,
        4.,
        crate::theme_color(theme.accent).gamma_multiply(if selected { 0.8 } else { 0.35 }),
    );
    ui.painter_at(rect).text(
        rect.left_center() + Vec2::new(6., 0.),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(12.),
        crate::theme_color(theme.text),
    );
}
#[derive(Default)]
pub struct PianoRollEditor {
    pub selected: Option<u32>,
    pub snap_beats: f64,
}
impl PianoRollEditor {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        document: &mut Document,
        clip_id: &str,
    ) -> Result<bool, soniclaw_core::Error> {
        document.validate()?;
        let clip = document
            .midi
            .iter()
            .find(|c| c.id == clip_id)
            .ok_or(soniclaw_core::Error::InvalidParameter)?;
        let width = ui.available_width().clamp(120., 8192.);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 240.), Sense::click());
        let theme = crate::current_theme(ui.ctx());
        ui.painter_at(rect)
            .rect_filled(rect, 4., crate::theme_color(theme.surface));
        let key_base = 48u8;
        let key_rows = 24.;
        let row = rect.height() / key_rows;
        for key in 0..24 {
            let y = rect.bottom() - key as f32 * row;
            ui.painter_at(rect).line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1., crate::theme_color(theme.muted).gamma_multiply(0.25)),
            );
        }
        let mut edit = None;
        for note in &clip.notes {
            if !(48..72).contains(&note.key) {
                continue;
            }
            let x = rect.left() + note.beat as f32 / clip.source_length as f32 * width;
            let y = rect.bottom() - (note.key - key_base + 1) as f32 * row;
            let box_rect = Rect::from_min_size(
                Pos2::new(x, y),
                Vec2::new(
                    (note.length as f32 / clip.source_length as f32 * width).max(4.),
                    row - 1.,
                ),
            );
            let r = ui.interact(
                box_rect,
                ui.id().with((clip_id, note.id)),
                Sense::click_and_drag(),
            );
            if crate::accessibility::selectable(
                ui,
                &r,
                &format!(
                    "Note {} pitch {} beat {:.2} length {:.2}",
                    note.id, note.key, note.beat, note.length
                ),
                self.selected == Some(note.id),
            ) {
                self.selected = Some(note.id);
                r.request_focus();
            }
            if r.has_focus() && ui.is_enabled() {
                let step = if self.snap_beats.is_finite() && self.snap_beats > 0. {
                    self.snap_beats
                } else {
                    0.25
                };
                let mut updated = note.clone();
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    edit = Some(Edit::RemoveNote {
                        clip: clip_id.into(),
                        note: note.id,
                    });
                } else {
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                        updated.beat = (note.beat + step).min(clip.source_length - note.length);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                        updated.beat = (note.beat - step).max(0.);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        updated.key = note.key.saturating_add(1).min(127);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        updated.key = note.key.saturating_sub(1);
                    }
                    if updated.beat != note.beat || updated.key != note.key {
                        edit = Some(Edit::UpsertNote {
                            clip: clip_id.into(),
                            note: updated,
                        });
                    }
                }
            }
            if r.drag_started() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(r.id, (note.beat, note.key)));
            }
            if r.dragged() {
                let (start, key) = ui
                    .ctx()
                    .data(|d| d.get_temp::<(f64, u8)>(r.id))
                    .unwrap_or((note.beat, note.key));
                let delta = r.total_drag_delta().unwrap_or_default();
                let value = (start + delta.x as f64 / width as f64 * clip.source_length)
                    .clamp(0., (clip.source_length - note.length).max(0.));
                let beat = if self.snap_beats > 0. && self.snap_beats.is_finite() {
                    (value / self.snap_beats).round() * self.snap_beats
                } else {
                    value
                };
                edit = Some(Edit::UpsertNote {
                    clip: clip_id.into(),
                    note: Note {
                        beat: beat.min(clip.source_length - note.length),
                        key: (key as i32 - (delta.y / row).round() as i32).clamp(0, 127) as u8,
                        ..note.clone()
                    },
                });
            }
            paint_clip(
                ui,
                box_rect.intersect(rect),
                &note.key.to_string(),
                self.selected == Some(note.id),
                &theme,
            );
        }
        ui.collapsing("Accessible note list", |ui| {
            for n in &clip.notes {
                if ui
                    .selectable_label(
                        self.selected == Some(n.id),
                        format!("Note {} · pitch {} · beat {:.2}", n.id, n.key, n.beat),
                    )
                    .clicked()
                {
                    self.selected = Some(n.id);
                }
            }
        });
        if ui.button("Add note at start").clicked() {
            let id = clip
                .notes
                .iter()
                .map(|n| n.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(soniclaw_core::Error::InvalidEvents)?;
            edit = Some(Edit::UpsertNote {
                clip: clip_id.into(),
                note: Note {
                    id,
                    beat: 0.,
                    length: 0.5f64.min(clip.source_length),
                    channel: 0,
                    key: 60,
                    velocity: 0.8,
                },
            });
            self.selected = Some(id);
        }
        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let beat = (((pos.x - rect.left()) / width) as f64 * clip.source_length)
                    .clamp(0., (clip.source_length - 0.000001).max(0.));
                let beat = if self.snap_beats > 0. {
                    (beat / self.snap_beats).floor() * self.snap_beats
                } else {
                    beat
                };
                let length = 0.5f64.min(clip.source_length - beat);
                let id = clip
                    .notes
                    .iter()
                    .map(|n| n.id)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(soniclaw_core::Error::InvalidEvents)?;
                edit = Some(Edit::UpsertNote {
                    clip: clip_id.into(),
                    note: Note {
                        id,
                        beat,
                        length,
                        channel: 0,
                        key: 48 + (((rect.bottom() - pos.y) / row) as u8).min(23),
                        velocity: 0.8,
                    },
                });
            }
        }
        let mut changed = false;
        if let Some(edit) = edit {
            document.apply(edit)?;
            changed = true;
        }
        if let Some(id) = self.selected {
            if let Some(note) = document
                .midi
                .iter()
                .find(|c| c.id == clip_id)
                .and_then(|c| c.notes.iter().find(|n| n.id == id))
                .cloned()
            {
                let source_length = document
                    .midi
                    .iter()
                    .find(|c| c.id == clip_id)
                    .unwrap()
                    .source_length;
                let mut updated = note.clone();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut updated.beat)
                            .prefix("Beat ")
                            .range(0.0..=(source_length - note.length).max(0.)),
                    );
                    ui.add(
                        egui::DragValue::new(&mut updated.key)
                            .prefix("MIDI pitch ")
                            .range(0..=127),
                    );
                    ui.label("Note length");
                    ui.add(
                        egui::DragValue::new(&mut updated.length)
                            .speed(0.25)
                            .range(0.0000001..=(source_length - note.beat).max(0.0000001)),
                    );
                    ui.label("Velocity");
                    ui.add(egui::Slider::new(&mut updated.velocity, 0.0..=1.0));
                });
                if updated.beat != note.beat
                    || updated.key != note.key
                    || updated.length != note.length
                    || updated.velocity != note.velocity
                {
                    document.apply(Edit::UpsertNote {
                        clip: clip_id.into(),
                        note: updated,
                    })?;
                    changed = true;
                }
                if ui.button("Delete note").clicked() {
                    document.apply(Edit::RemoveNote {
                        clip: clip_id.into(),
                        note: id,
                    })?;
                    self.selected = None;
                    changed = true;
                }
            }
        }
        Ok(changed)
    }
}
#[cfg(test)]
mod accessibility_tests {
    use super::*;
    #[test]
    fn focused_note_keyboard_edit_changes_valid_document() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut d = Document {
            schema: 1,
            sample_rate: 48000,
            channels: 2,
            timeline: Default::default(),
            audio: vec![],
            midi: vec![soniclaw_clips::MidiClip {
                controls: vec![],
                id: "melody".into(),
                track: 0,
                start_beat: 0.,
                length_beats: 4.,
                source_offset: 0.,
                source_length: 4.,
                looping: false,
                enabled: true,
                notes: vec![Note {
                    id: 1,
                    beat: 0.,
                    length: 0.5,
                    channel: 0,
                    key: 60,
                    velocity: 0.8,
                }],
            }],
        };
        let mut editor = PianoRollEditor {
            selected: Some(1),
            snap_beats: 0.25,
        };
        let mut note_id = egui::Id::NULL;
        let mut first = ctx.run_ui(Default::default(), |ui| {
            note_id = ui.id().with(("melody", 1u32));
            editor.show(ui, &mut d, "melody").unwrap();
        });
        first.textures_delta.clear();
        assert!(first
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .any(|(_, n)| n.label().is_some_and(|s| s.starts_with("Note 1 pitch 60"))));
        ctx.memory_mut(|m| m.request_focus(note_id));
        for (key, pitch, beat) in [
            (egui::Key::ArrowUp, 61, 0.),
            (egui::Key::ArrowRight, 61, 0.25),
        ] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ui| {
                    editor.show(ui, &mut d, "melody").unwrap();
                },
            );
            out.textures_delta.clear();
            assert_eq!(d.midi[0].notes[0].key, pitch);
            assert_eq!(d.midi[0].notes[0].beat, beat);
        }
        d.validate().unwrap();
    }
}
