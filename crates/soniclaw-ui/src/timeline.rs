//! Virtualized arrangement/piano-roll views and transactional multi-selection editing.
use egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use soniclaw_clips::{Document, Note};
use soniclaw_commands::History;
use soniclaw_core::Error;
use std::collections::BTreeSet;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub schema: u32,
    pub frames_per_pixel: f64,
    pub beats_per_pixel: f64,
    pub key_height: f32,
    pub track_heights: Vec<f32>,
    pub arrangement_scroll: [f32; 2],
    pub piano_scroll: [f32; 2],
    pub snap_frames: u64,
    pub snap_beats: f64,
}
impl Default for View {
    fn default() -> Self {
        Self {
            schema: 1,
            frames_per_pixel: 240.,
            beats_per_pixel: 0.0125,
            key_height: 16.,
            track_heights: vec![48.; 64],
            arrangement_scroll: [0.; 2],
            piano_scroll: [0., 55. * 16.],
            snap_frames: 6000,
            snap_beats: 0.25,
        }
    }
}
impl View {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !self.frames_per_pixel.is_finite()
            || !(1.0..=192000.).contains(&self.frames_per_pixel)
            || !self.beats_per_pixel.is_finite()
            || !(0.0001..=4.).contains(&self.beats_per_pixel)
            || !self.key_height.is_finite()
            || !(8.0..=40.).contains(&self.key_height)
            || self.track_heights.len() != 64
            || self
                .track_heights
                .iter()
                .any(|h| !h.is_finite() || !(24.0..=180.).contains(h))
            || self
                .arrangement_scroll
                .iter()
                .chain(&self.piano_scroll)
                .any(|x| !x.is_finite() || !(0.0..=1e7).contains(x))
            || !self.snap_beats.is_finite()
            || !(0.0..=1024.).contains(&self.snap_beats)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_json(text: &str) -> Result<Self, Error> {
        if text.len() > 8192 {
            return Err(Error::InvalidParameter);
        }
        let s: Self = serde_json::from_str(text).map_err(|_| Error::InvalidParameter)?;
        s.validate()?;
        Ok(s)
    }
}
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub clips: BTreeSet<String>,
    pub notes: BTreeSet<(String, u32)>,
}
#[derive(Clone, Debug)]
pub enum Action {
    MoveClips { frames: i64, tracks: i16 },
    MoveNotes { beats: f64, semitones: i16 },
    DeleteClips,
    DeleteNotes,
}
pub struct TimelineEditor {
    /// Hide local undo buttons when an application supplies project-wide history.
    pub show_history_controls: bool,
    pub view: View,
    pub selection: Selection,
    history: History<Document>,
    pub last_visible: usize,
    tools: AudioTools,
}
impl Default for TimelineEditor {
    fn default() -> Self {
        Self {
            show_history_controls: true,
            view: View::default(),
            selection: Selection::default(),
            history: History::new(16).expect("bounded history"),
            last_visible: 0,
            tools: AudioTools::default(),
        }
    }
}
fn frame_of(beat: f64, map: &soniclaw_transport::TempoMap, rate: u32) -> Result<u64, Error> {
    Ok((map.seconds_at(beat)? * rate as f64).round() as u64)
}
impl TimelineEditor {
    pub fn clear_history(&mut self) {
        self.history = History::new(16).expect("bounded history");
        self.selection = Selection::default();
    }
    pub fn undo(&mut self, d: &mut Document) -> bool {
        self.history.undo(d)
    }
    pub fn redo(&mut self, d: &mut Document) -> bool {
        self.history.redo(d)
    }
    /// Apply a whole selection transactionally. A failed validation leaves document/history unchanged.
    pub fn apply(&mut self, d: &mut Document, action: Action) -> Result<(), Error> {
        self.view.validate()?;
        d.validate()?;
        let mut selected = self.selection.clone();
        let groups = d
            .audio
            .iter()
            .filter(|c| selected.clips.contains(&c.id))
            .filter_map(|c| c.edits.as_ref().and_then(|m| m.link_group.clone()))
            .collect::<BTreeSet<_>>();
        for c in &d.audio {
            if c.edits
                .as_ref()
                .and_then(|m| m.link_group.as_ref())
                .is_some_and(|g| groups.contains(g))
            {
                selected.clips.insert(c.id.clone());
            }
        }
        let frames_snap = self.view.snap_frames;
        let beat_snap = self.view.snap_beats;
        self.history.edit(d, |next| {
            match action {
                Action::MoveClips { frames, tracks } => {
                    let map = soniclaw_transport::TempoMap::compile(&next.timeline)?;
                    let mut found = 0;
                    let mut group_delta = std::collections::BTreeMap::new();
                    for c in &next.audio {
                        if selected.clips.contains(&c.id) {
                            if let Some(group) =
                                c.edits.as_ref().and_then(|m| m.link_group.as_ref())
                            {
                                if !group_delta.contains_key(group) {
                                    let mut target = c
                                        .start_frame
                                        .checked_add_signed(frames)
                                        .ok_or(Error::InvalidParameter)?;
                                    if frames_snap > 0 {
                                        target = ((target as f64 / frames_snap as f64).round()
                                            * frames_snap as f64)
                                            as u64;
                                    }
                                    group_delta.insert(
                                        group.clone(),
                                        target as i128 - c.start_frame as i128,
                                    );
                                }
                            }
                        }
                    }
                    for c in &mut next.audio {
                        if selected.clips.contains(&c.id) {
                            if let Some(delta) = c
                                .edits
                                .as_ref()
                                .and_then(|m| m.link_group.as_ref())
                                .and_then(|g| group_delta.get(g))
                            {
                                c.start_frame = u64::try_from(c.start_frame as i128 + *delta)
                                    .map_err(|_| Error::InvalidParameter)?;
                            } else {
                                c.start_frame = c
                                    .start_frame
                                    .checked_add_signed(frames)
                                    .ok_or(Error::InvalidParameter)?;
                                if frames_snap > 0 {
                                    c.start_frame = ((c.start_frame as f64 / frames_snap as f64)
                                        .round()
                                        * frames_snap as f64)
                                        as u64;
                                }
                            }
                            c.track = c
                                .track
                                .checked_add_signed(tracks)
                                .filter(|t| *t < 64)
                                .ok_or(Error::InvalidParameter)?;
                            found += 1;
                        }
                    }
                    for c in &mut next.midi {
                        if selected.clips.contains(&c.id) {
                            let at = frame_of(c.start_beat, &map, next.sample_rate)?
                                .checked_add_signed(frames)
                                .ok_or(Error::InvalidParameter)?;
                            c.start_beat =
                                map.beat_at_seconds(at as f64 / next.sample_rate as f64)?;
                            if beat_snap > 0. {
                                c.start_beat = (c.start_beat / beat_snap).round() * beat_snap;
                            }
                            c.track = c
                                .track
                                .checked_add_signed(tracks)
                                .filter(|t| *t < 64)
                                .ok_or(Error::InvalidParameter)?;
                            found += 1;
                        }
                    }
                    if found != selected.clips.len() {
                        return Err(Error::InvalidParameter);
                    }
                }
                Action::MoveNotes { beats, semitones } => {
                    if !beats.is_finite() {
                        return Err(Error::InvalidParameter);
                    }
                    let mut found = 0;
                    for c in &mut next.midi {
                        for n in &mut c.notes {
                            if selected.notes.contains(&(c.id.clone(), n.id)) {
                                n.beat += beats;
                                if beat_snap > 0. {
                                    n.beat = (n.beat / beat_snap).round() * beat_snap;
                                }
                                n.key = n
                                    .key
                                    .checked_add_signed(
                                        i8::try_from(semitones)
                                            .map_err(|_| Error::InvalidParameter)?,
                                    )
                                    .filter(|k| *k < 128)
                                    .ok_or(Error::InvalidParameter)?;
                                found += 1;
                            }
                        }
                    }
                    if found != selected.notes.len() {
                        return Err(Error::InvalidParameter);
                    }
                }
                Action::DeleteClips => {
                    next.audio.retain(|c| !selected.clips.contains(&c.id));
                    next.midi.retain(|c| !selected.clips.contains(&c.id));
                }
                Action::DeleteNotes => {
                    for c in &mut next.midi {
                        c.notes
                            .retain(|n| !selected.notes.contains(&(c.id.clone(), n.id)));
                    }
                }
            }
            next.validate()
        })
    }
    /// Select intersecting model regions, independent of the painted viewport.
    pub fn select_regions(
        &mut self,
        d: &Document,
        start: u64,
        end: u64,
        first_track: u16,
        last_track: u16,
        extend: bool,
    ) -> Result<(), Error> {
        d.validate()?;
        if start > end || first_track > last_track {
            return Err(Error::InvalidParameter);
        }
        if !extend {
            self.selection.clips.clear();
        }
        let map = soniclaw_transport::TempoMap::compile(&d.timeline)?;
        for c in &d.audio {
            if (first_track..=last_track).contains(&c.track)
                && c.start_frame < end
                && c.start_frame + c.length_frames > start
            {
                self.selection.clips.insert(c.id.clone());
            }
        }
        for c in &d.midi {
            if (first_track..=last_track).contains(&c.track)
                && frame_of(c.start_beat, &map, d.sample_rate)? < end
                && frame_of(c.start_beat + c.length_beats, &map, d.sample_rate)? > start
            {
                self.selection.clips.insert(c.id.clone());
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn select_notes(
        &mut self,
        d: &Document,
        clip: &str,
        start: f64,
        end: f64,
        low: u8,
        high: u8,
        extend: bool,
    ) -> Result<(), Error> {
        let c = d
            .midi
            .iter()
            .find(|c| c.id == clip)
            .ok_or(Error::InvalidParameter)?;
        if !start.is_finite() || !end.is_finite() || start > end || low > high {
            return Err(Error::InvalidParameter);
        }
        if !extend {
            self.selection.notes.clear();
        }
        for n in &c.notes {
            if (low..=high).contains(&n.key) && n.beat < end && n.beat + n.length > start {
                self.selection.notes.insert((clip.into(), n.id));
            }
        }
        Ok(())
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, d: &mut Document) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            if self.show_history_controls
                && ui
                    .add_enabled(
                        self.history.can_undo(),
                        egui::Button::new("Undo editor edit"),
                    )
                    .clicked()
            {
                changed |= self.undo(d);
            }
            if self.show_history_controls
                && ui
                    .add_enabled(
                        self.history.can_redo(),
                        egui::Button::new("Redo editor edit"),
                    )
                    .clicked()
            {
                changed |= self.redo(d);
            }
            ui.label("Shift/Ctrl to extend selection · drag blank space for marquee");
        });
        changed
    }
    pub fn arrangement(
        &mut self,
        ui: &mut egui::Ui,
        d: &mut Document,
        tracks: usize,
    ) -> Result<bool, Error> {
        self.view.validate()?;
        d.validate()?;
        if !(1..=64).contains(&tracks) {
            return Err(Error::InvalidParameter);
        }
        let mut changed = self.toolbar(ui, d);
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut self.view.frames_per_pixel, 1.0..=192000.)
                    .logarithmic(true)
                    .text("Frames / pixel"),
            );
            ui.add(egui::DragValue::new(&mut self.view.snap_frames).prefix("Snap frames "));
        });
        let map = soniclaw_transport::TempoMap::compile(&d.timeline)?;
        let mut regions = Vec::new();
        for c in &d.audio {
            regions.push((c.id.clone(), c.track, c.start_frame, c.length_frames));
        }
        for c in &d.midi {
            let start = frame_of(c.start_beat, &map, d.sample_rate)?;
            let end = frame_of(c.start_beat + c.length_beats, &map, d.sample_rate)?;
            regions.push((c.id.clone(), c.track, start, end - start));
        }
        let end = regions
            .iter()
            .map(|r| r.2 + r.3)
            .max()
            .unwrap_or(d.sample_rate as u64 * 10);
        let width = ((end as f64 / self.view.frames_per_pixel + 300.) as f32)
            .clamp(ui.available_width().max(100.), 1e7);
        let mut ys = vec![0.];
        for h in &self.view.track_heights[..tracks] {
            ys.push(ys.last().unwrap() + h);
        }
        let total_y = *ys.last().unwrap();
        let mut command = None;
        let mut visible = 0;
        let output = egui::ScrollArea::both()
            .id_salt("scalable-arrangement")
            .max_height(320.)
            .scroll_offset(Vec2::new(
                self.view.arrangement_scroll[0],
                self.view.arrangement_scroll[1],
            ))
            .show_viewport(ui, |ui, viewport| {
                let (rect, background) =
                    ui.allocate_exact_size(Vec2::new(width, total_y), Sense::click_and_drag());
                let painter = ui.painter_at(ui.clip_rect());
                let mut dragging = false;
                for track in 0..tracks {
                    let y = rect.top() + ys[track];
                    let row = Rect::from_min_size(
                        Pos2::new(rect.left(), y),
                        Vec2::new(width, self.view.track_heights[track]),
                    );
                    if !row.intersects(ui.clip_rect()) {
                        continue;
                    }
                    painter.rect_stroke(
                        row,
                        0.,
                        Stroke::new(1., egui::Color32::DARK_GRAY),
                        egui::StrokeKind::Inside,
                    );
                    painter.text(
                        row.left_top() + Vec2::splat(3.),
                        Align2::LEFT_TOP,
                        format!("Track {}", track + 1),
                        FontId::proportional(11.),
                        egui::Color32::GRAY,
                    );
                    let handle = Rect::from_min_size(
                        Pos2::new(rect.left(), row.bottom() - 4.),
                        Vec2::new(viewport.width(), 4.),
                    );
                    let r =
                        ui.interact(handle, ui.id().with(("track-height", track)), Sense::drag());
                    if r.dragged() {
                        self.view.track_heights[track] = (self.view.track_heights[track]
                            + ui.input(|i| i.pointer.delta().y))
                        .clamp(24., 180.);
                        dragging = true;
                    }
                }
                for (id, track, start, length) in &regions {
                    if *track as usize >= tracks {
                        continue;
                    }
                    let pos = Pos2::new(
                        rect.left() + (*start as f64 / self.view.frames_per_pixel) as f32,
                        rect.top() + ys[*track as usize] + 15.,
                    );
                    let r = Rect::from_min_size(
                        pos,
                        Vec2::new(
                            (*length as f64 / self.view.frames_per_pixel) as f32,
                            self.view.track_heights[*track as usize] - 20.,
                        ),
                    );
                    if !r.intersects(ui.clip_rect()) {
                        continue;
                    }
                    visible += 1;
                    let response =
                        ui.interact(r, ui.id().with(("region", id)), Sense::click_and_drag());
                    let extend =
                        ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
                    if response.clicked() || response.drag_started() {
                        if !extend && !self.selection.clips.contains(id) {
                            self.selection.clips.clear();
                        }
                        if extend && response.clicked() && self.selection.clips.contains(id) {
                            self.selection.clips.remove(id);
                        } else {
                            self.selection.clips.insert(id.clone());
                        }
                        response.request_focus();
                    }
                    let assistive = crate::accessibility::selectable(
                        ui,
                        &response,
                        &format!("Clip {id}, track {}, frame {start}", track + 1),
                        self.selection.clips.contains(id),
                    );
                    if assistive && !response.clicked() && !response.drag_started() {
                        self.selection.clips.insert(id.clone());
                    }
                    let selected = self.selection.clips.contains(id);
                    painter.rect_filled(
                        r,
                        4.,
                        if selected {
                            egui::Color32::from_rgb(35, 155, 130)
                        } else {
                            egui::Color32::from_rgb(45, 70, 100)
                        },
                    );
                    painter.text(
                        r.left_center() + Vec2::new(4., 0.),
                        Align2::LEFT_CENTER,
                        id,
                        FontId::proportional(12.),
                        egui::Color32::WHITE,
                    );
                    if response.dragged() {
                        dragging = true;
                        let delta = response.total_drag_delta().unwrap_or_default();
                        painter.rect_stroke(
                            r.translate(delta),
                            4.,
                            Stroke::new(2., egui::Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if response.drag_stopped() {
                        let delta = response.total_drag_delta().unwrap_or_default();
                        command = Some(Action::MoveClips {
                            frames: (delta.x as f64 * self.view.frames_per_pixel).round() as i64,
                            tracks: ys
                                .windows(2)
                                .position(|row| r.center().y + delta.y - rect.top() < row[1])
                                .unwrap_or(tracks - 1) as i16
                                - *track as i16,
                        });
                    }
                    if response.has_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                            command = Some(Action::DeleteClips);
                        } else {
                            let direction = ui.input(|i| {
                                if i.key_pressed(egui::Key::ArrowRight) {
                                    1
                                } else if i.key_pressed(egui::Key::ArrowLeft) {
                                    -1
                                } else {
                                    0
                                }
                            });
                            if direction != 0 {
                                command = Some(Action::MoveClips {
                                    frames: direction * self.view.snap_frames.max(1) as i64,
                                    tracks: 0,
                                });
                            }
                        }
                    }
                }
                if background.drag_started() {
                    if let Some(pos) = background.interact_pointer_pos() {
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                background.id,
                                pos - background.total_drag_delta().unwrap_or_default(),
                            )
                        });
                    }
                }
                if background.dragged() && !dragging {
                    if let (Some(anchor), Some(pos)) = (
                        ui.ctx().data(|data| data.get_temp::<Pos2>(background.id)),
                        background.interact_pointer_pos(),
                    ) {
                        painter.rect_stroke(
                            Rect::from_two_pos(anchor, pos),
                            0.,
                            Stroke::new(1., egui::Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if background.drag_stopped() && !dragging {
                    if let (Some(anchor), Some(pos)) = (
                        ui.ctx().data(|data| data.get_temp::<Pos2>(background.id)),
                        background.interact_pointer_pos(),
                    ) {
                        let area = Rect::from_two_pos(anchor, pos);
                        let first = ys
                            .windows(2)
                            .position(|y| area.top() - rect.top() < y[1])
                            .unwrap_or(tracks - 1);
                        let last = ys
                            .windows(2)
                            .rposition(|y| area.bottom() - rect.top() >= y[0])
                            .unwrap_or(0);
                        self.select_regions(
                            d,
                            ((area.left() - rect.left()).max(0.) as f64
                                * self.view.frames_per_pixel) as u64,
                            ((area.right() - rect.left()).max(0.) as f64
                                * self.view.frames_per_pixel) as u64,
                            first as u16,
                            last as u16,
                            ui.input(|i| i.modifiers.shift),
                        )
                        .expect("validated region selection");
                    }
                }
            });
        self.view.arrangement_scroll = [output.state.offset.x, output.state.offset.y];
        self.last_visible = visible;
        if let Some(action) = command {
            self.apply(d, action)?;
            changed = true;
        }
        Ok(changed)
    }
    pub fn piano_roll(
        &mut self,
        ui: &mut egui::Ui,
        d: &mut Document,
        clip: &str,
    ) -> Result<bool, Error> {
        self.view.validate()?;
        d.validate()?;
        let mut changed = self.toolbar(ui, d);
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut self.view.beats_per_pixel, 0.0001..=4.)
                    .logarithmic(true)
                    .text("Beats / pixel"),
            );
            ui.add(egui::Slider::new(&mut self.view.key_height, 8.0..=40.).text("Key height"));
            ui.add(
                egui::DragValue::new(&mut self.view.snap_beats)
                    .prefix("Note snap ")
                    .range(0.0..=1024.),
            );
        });
        let c = d
            .midi
            .iter()
            .find(|c| c.id == clip)
            .ok_or(Error::InvalidParameter)?;
        let notes = c.notes.clone();
        let source_length = c.source_length;
        let width = (source_length / self.view.beats_per_pixel + 90.) as f32;
        let width = width.clamp(ui.available_width().max(100.), 1e7);
        let mut command = None;
        let mut visible = 0;
        let mut add = false;
        ui.horizontal(|ui| {
            add = ui.button("Add middle C").clicked();
            if ui.button("Select all notes").clicked() {
                self.selection.notes = notes.iter().map(|n| (clip.to_string(), n.id)).collect();
            }
        });
        let output = egui::ScrollArea::both()
            .id_salt(("scalable-piano", clip))
            .max_height(300.)
            .scroll_offset(Vec2::new(
                self.view.piano_scroll[0],
                self.view.piano_scroll[1],
            ))
            .show_viewport(ui, |ui, _viewport| {
                let (rect, background) = ui.allocate_exact_size(
                    Vec2::new(width, 128. * self.view.key_height),
                    Sense::click_and_drag(),
                );
                let painter = ui.painter_at(ui.clip_rect());
                let mut dragging = false;
                for key in 0u8..128 {
                    let row = Rect::from_min_size(
                        Pos2::new(
                            rect.left(),
                            rect.top() + (127 - key) as f32 * self.view.key_height,
                        ),
                        Vec2::new(width, self.view.key_height),
                    );
                    if row.intersects(ui.clip_rect()) {
                        painter.rect_filled(
                            row,
                            0.,
                            if [1, 3, 6, 8, 10].contains(&(key % 12)) {
                                egui::Color32::from_gray(25)
                            } else {
                                egui::Color32::from_gray(40)
                            },
                        );
                        painter.text(
                            row.left_center(),
                            Align2::LEFT_CENTER,
                            key.to_string(),
                            FontId::proportional(10.),
                            egui::Color32::GRAY,
                        );
                    }
                }
                for note in &notes {
                    let pos = Pos2::new(
                        rect.left() + note.beat as f32 / self.view.beats_per_pixel as f32,
                        rect.top() + (127 - note.key) as f32 * self.view.key_height,
                    );
                    let r = Rect::from_min_size(
                        pos,
                        Vec2::new(
                            (note.length / self.view.beats_per_pixel) as f32,
                            self.view.key_height - 1.,
                        ),
                    );
                    if !r.intersects(ui.clip_rect()) {
                        continue;
                    }
                    visible += 1;
                    let response =
                        ui.interact(r, ui.id().with((clip, note.id)), Sense::click_and_drag());
                    let identity = (clip.to_string(), note.id);
                    let extend =
                        ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command);
                    if response.clicked() || response.drag_started() {
                        if !extend && !self.selection.notes.contains(&identity) {
                            self.selection.notes.clear();
                        }
                        if extend && response.clicked() && self.selection.notes.contains(&identity)
                        {
                            self.selection.notes.remove(&identity);
                        } else {
                            self.selection.notes.insert(identity.clone());
                        }
                        response.request_focus();
                    }
                    let assistive = crate::accessibility::selectable(
                        ui,
                        &response,
                        &format!("Note {} pitch {} beat {:.2}", note.id, note.key, note.beat),
                        self.selection.notes.contains(&identity),
                    );
                    if assistive && !response.clicked() && !response.drag_started() {
                        self.selection.notes.insert(identity.clone());
                    }
                    painter.rect_filled(
                        r,
                        2.,
                        if self.selection.notes.contains(&identity) {
                            egui::Color32::from_rgb(220, 150, 50)
                        } else {
                            egui::Color32::from_rgb(60, 125, 180)
                        },
                    );
                    if response.dragged() {
                        dragging = true;
                        let delta = response.total_drag_delta().unwrap_or_default();
                        painter.rect_stroke(
                            r.translate(delta),
                            2.,
                            Stroke::new(1., egui::Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                    }
                    if response.drag_stopped() {
                        let delta = response.total_drag_delta().unwrap_or_default();
                        command = Some(Action::MoveNotes {
                            beats: delta.x as f64 * self.view.beats_per_pixel,
                            semitones: -(delta.y / self.view.key_height).round() as i16,
                        });
                    }
                    if response.has_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                            command = Some(Action::DeleteNotes);
                        } else {
                            let direction = ui.input(|i| {
                                if i.key_pressed(egui::Key::ArrowRight) {
                                    1.
                                } else if i.key_pressed(egui::Key::ArrowLeft) {
                                    -1.
                                } else {
                                    0.
                                }
                            });
                            let pitch = ui.input(|i| {
                                if i.key_pressed(egui::Key::ArrowUp) {
                                    1
                                } else if i.key_pressed(egui::Key::ArrowDown) {
                                    -1
                                } else {
                                    0
                                }
                            });
                            if direction != 0. || pitch != 0 {
                                command = Some(Action::MoveNotes {
                                    beats: direction
                                        * if self.view.snap_beats > 0. {
                                            self.view.snap_beats
                                        } else {
                                            0.01
                                        },
                                    semitones: pitch,
                                });
                            }
                        }
                    }
                }
                if background.drag_started() {
                    if let Some(pos) = background.interact_pointer_pos() {
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                background.id,
                                pos - background.total_drag_delta().unwrap_or_default(),
                            )
                        });
                    }
                }
                if background.dragged() && !dragging {
                    if let (Some(anchor), Some(pos)) = (
                        ui.ctx().data(|data| data.get_temp::<Pos2>(background.id)),
                        background.interact_pointer_pos(),
                    ) {
                        painter.rect_stroke(
                            Rect::from_two_pos(anchor, pos),
                            0.,
                            Stroke::new(1., egui::Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if background.drag_stopped() && !dragging {
                    if let (Some(anchor), Some(pos)) = (
                        ui.ctx().data(|data| data.get_temp::<Pos2>(background.id)),
                        background.interact_pointer_pos(),
                    ) {
                        let area = Rect::from_two_pos(anchor, pos);
                        let low = (127. - ((area.bottom() - rect.top()) / self.view.key_height))
                            .floor()
                            .clamp(0., 127.) as u8;
                        let high = (127. - ((area.top() - rect.top()) / self.view.key_height))
                            .ceil()
                            .clamp(0., 127.) as u8;
                        self.select_notes(
                            d,
                            clip,
                            ((area.left() - rect.left()).max(0.) as f64
                                * self.view.beats_per_pixel)
                                .min(source_length),
                            ((area.right() - rect.left()).max(0.) as f64
                                * self.view.beats_per_pixel)
                                .min(source_length),
                            low,
                            high,
                            ui.input(|i| i.modifiers.shift),
                        )
                        .expect("validated note selection");
                    }
                }
            });
        self.view.piano_scroll = [output.state.offset.x, output.state.offset.y];
        self.last_visible = visible;
        if add {
            let mut next = d.clone();
            let c = next
                .midi
                .iter_mut()
                .find(|c| c.id == clip)
                .ok_or(Error::InvalidParameter)?;
            let id = c
                .notes
                .iter()
                .map(|n| n.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(Error::InvalidParameter)?;
            c.notes.push(Note {
                id,
                beat: 0.,
                length: 0.5f64.min(c.source_length),
                channel: 0,
                key: 60,
                velocity: 0.8,
            });
            next.validate()?;
            self.history.record(std::mem::replace(d, next), "Add note");
            changed = true;
        }
        if let Some(action) = command {
            self.apply(d, action)?;
            changed = true;
        }
        Ok(changed)
    }
}
#[derive(Default)]
struct AudioTools {
    loaded: Option<String>,
    split: u64,
    new_id: String,
    slip: i64,
    group: String,
    shape: soniclaw_clips::FadeShape,
    points: Vec<soniclaw_clips::GainPoint>,
    point_frame: u64,
    point_gain: f32,
}
impl TimelineEditor {
    /// Apply one shared core edit through this editor's transactional undo history.
    pub fn apply_audio(
        &mut self,
        d: &mut Document,
        edit: soniclaw_clips::Edit,
    ) -> Result<(), Error> {
        self.history.edit(d, |candidate| {
            candidate.apply(edit)?;
            Ok(())
        })
    }
    /// Reusable selected-region split/join/slip/crossfade/link/envelope controls.
    pub fn audio_tools(&mut self, ui: &mut egui::Ui, d: &mut Document) -> Result<bool, Error> {
        use soniclaw_clips::{Edit, FadeShape, GainPoint};
        let selected = d
            .audio
            .iter()
            .filter(|c| self.selection.clips.contains(&c.id))
            .cloned()
            .collect::<Vec<_>>();
        let Some(c) = selected.first() else {
            return Ok(false);
        };
        if self.tools.loaded.as_ref() != Some(&c.id) {
            self.tools.loaded = Some(c.id.clone());
            self.tools.split = c.start_frame + c.length_frames / 2;
            self.tools.new_id = format!("{}.split", c.id);
            self.tools.slip = 0;
            self.tools.group = c
                .edits
                .as_ref()
                .and_then(|m| m.link_group.clone())
                .unwrap_or_else(|| "linked".into());
            self.tools.shape = c.edits.as_ref().map_or(FadeShape::Linear, |m| m.fade_shape);
            self.tools.points = if let Some(m) = &c.edits {
                let mut points = vec![GainPoint {
                    frame: 0,
                    gain: m.envelope_value(0),
                }];
                points.extend(
                    m.envelope
                        .iter()
                        .filter(|p| {
                            p.frame > m.envelope_offset
                                && p.frame < m.envelope_offset + c.length_frames
                        })
                        .map(|p| GainPoint {
                            frame: p.frame - m.envelope_offset,
                            gain: p.gain,
                        }),
                );
                points.push(GainPoint {
                    frame: c.length_frames,
                    gain: m.envelope_value(c.length_frames),
                });
                points
            } else {
                vec![
                    GainPoint { frame: 0, gain: 1. },
                    GainPoint {
                        frame: c.length_frames,
                        gain: 1.,
                    },
                ]
            };
        }
        let mut edit = None;
        egui::CollapsingHeader::new("Nondestructive audio edits")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!("Selected audio region: {}", c.id));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.tools.split)
                            .prefix("Split at frame ")
                            .range(c.start_frame..=c.start_frame + c.length_frames),
                    );
                    ui.text_edit_singleline(&mut self.tools.new_id);
                    if ui.button("Split audio").clicked() {
                        edit = Some(Edit::SplitAudio {
                            id: c.id.clone(),
                            new_id: self.tools.new_id.clone(),
                            frame: self.tools.split,
                        });
                    }
                    if ui
                        .add_enabled(
                            selected.len() == 2,
                            egui::Button::new("Join compatible selection"),
                        )
                        .clicked()
                    {
                        let mut pair = selected.clone();
                        pair.sort_by_key(|c| c.start_frame);
                        edit = Some(Edit::JoinAudio {
                            left: pair[0].id.clone(),
                            right: pair[1].id.clone(),
                        });
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.tools.slip).prefix("Source slip frames "),
                    );
                    if ui.button("Apply source slip").clicked() {
                        edit = Some(Edit::SlipAudio {
                            id: c.id.clone(),
                            frames: self.tools.slip,
                        });
                    }
                    egui::ComboBox::from_id_salt("audio-fade-shape")
                        .selected_text(format!("{:?}", self.tools.shape))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.tools.shape,
                                FadeShape::Linear,
                                "Linear amplitude",
                            );
                            ui.selectable_value(
                                &mut self.tools.shape,
                                FadeShape::EqualPower,
                                "Equal power",
                            );
                        });
                    if ui
                        .add_enabled(selected.len() == 2, egui::Button::new("Crossfade overlap"))
                        .clicked()
                    {
                        let mut pair = selected.clone();
                        pair.sort_by_key(|c| c.start_frame);
                        edit = Some(Edit::CrossfadeAudio {
                            left: pair[0].id.clone(),
                            right: pair[1].id.clone(),
                            shape: self.tools.shape,
                        });
                    }
                });
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.tools.group);
                    if ui.button("Link selected audio").clicked() {
                        edit = Some(Edit::LinkAudio {
                            ids: selected.iter().map(|c| c.id.clone()).collect(),
                            group: Some(self.tools.group.clone()),
                        });
                    }
                    if ui.button("Unlink selected audio").clicked() {
                        edit = Some(Edit::LinkAudio {
                            ids: selected.iter().map(|c| c.id.clone()).collect(),
                            group: None,
                        });
                    }
                });
                ui.label("Region gain envelope (relative frames; independent of source slip)");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.tools.point_frame)
                            .prefix("Point frame ")
                            .range(0..=c.length_frames),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.tools.point_gain)
                            .prefix("Gain ")
                            .range(0.0..=8.0)
                            .speed(0.01),
                    );
                    if ui.button("Add / replace gain point").clicked() {
                        if let Some(point) = self
                            .tools
                            .points
                            .iter_mut()
                            .find(|p| p.frame == self.tools.point_frame)
                        {
                            point.gain = self.tools.point_gain;
                        } else if self.tools.points.len() < 1024 {
                            self.tools.points.push(GainPoint {
                                frame: self.tools.point_frame,
                                gain: self.tools.point_gain,
                            });
                            self.tools.points.sort_by_key(|p| p.frame);
                        }
                    }
                    if ui.button("Reset envelope draft").clicked() {
                        self.tools.points = vec![
                            GainPoint { frame: 0, gain: 1. },
                            GainPoint {
                                frame: c.length_frames,
                                gain: 1.,
                            },
                        ];
                    }
                });
                let width = ui.available_width().clamp(100., 4096.);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 70.), Sense::hover());
                let peak = self
                    .tools
                    .points
                    .iter()
                    .map(|p| p.gain)
                    .fold(1f32, f32::max);
                let points = self
                    .tools
                    .points
                    .iter()
                    .map(|p| {
                        Pos2::new(
                            rect.left() + p.frame as f32 / c.length_frames as f32 * width,
                            rect.bottom() - p.gain / peak * rect.height(),
                        )
                    })
                    .collect::<Vec<_>>();
                if points.len() > 1 {
                    ui.painter().add(egui::Shape::line(
                        points,
                        Stroke::new(2., egui::Color32::LIGHT_GREEN),
                    ));
                }
                let mut remove = None;
                egui::ScrollArea::vertical()
                    .id_salt("gain-points")
                    .max_height(120.)
                    .show_rows(ui, 24., self.tools.points.len(), |ui, range| {
                        for index in range {
                            ui.push_id(index, |ui| {
                                ui.horizontal(|ui| {
                                    let p = &mut self.tools.points[index];
                                    ui.add(
                                        egui::DragValue::new(&mut p.frame)
                                            .prefix("Frame ")
                                            .range(0..=c.length_frames),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut p.gain)
                                            .prefix("Gain ")
                                            .range(0.0..=8.0)
                                            .speed(0.01),
                                    );
                                    if ui.button("Remove point").clicked() {
                                        remove = Some(index);
                                    }
                                });
                            });
                        }
                    });
                if let Some(index) = remove {
                    self.tools.points.remove(index);
                }
                if ui.button("Apply gain envelope").clicked() {
                    let mut points = self.tools.points.clone();
                    points.sort_by_key(|p| p.frame);
                    edit = Some(Edit::GainEnvelope {
                        id: c.id.clone(),
                        points,
                    });
                }
            });
        if let Some(edit) = edit {
            self.apply_audio(d, edit)?;
            self.tools.loaded = None;
            self.selection
                .clips
                .retain(|id| d.audio.iter().any(|c| &c.id == id));
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> Document {
        Document {
            schema: 1,
            sample_rate: 48000,
            channels: 2,
            timeline: Default::default(),
            audio: vec![soniclaw_clips::AudioClip {
                edits: None,
                id: "a".into(),
                asset: "source".into(),
                track: 12,
                start_frame: 48000,
                length_frames: 48000,
                source_offset: 0,
                source_length: 48000,
                looping: false,
                gain: 1.,
                fade_in: 0,
                fade_out: 0,
                enabled: true,
            }],
            midi: vec![soniclaw_clips::MidiClip {
                controls: vec![],
                id: "m".into(),
                track: 12,
                start_beat: 2.,
                length_beats: 4.,
                source_offset: 0.,
                source_length: 4.,
                looping: false,
                notes: vec![
                    Note {
                        id: 1,
                        beat: 0.,
                        length: 0.5,
                        channel: 0,
                        key: 0,
                        velocity: 0.8,
                    },
                    Note {
                        id: 2,
                        beat: 1.,
                        length: 0.5,
                        channel: 0,
                        key: 127,
                        velocity: 0.8,
                    },
                ],
                enabled: true,
            }],
        }
    }
    #[test]
    fn multi_selection_history_full_range_and_atomic_failure() {
        let mut d = fixture();
        let original = d.to_json().unwrap();
        let mut e = TimelineEditor::default();
        e.select_regions(&d, 48000, 100000, 12, 12, false).unwrap();
        assert_eq!(e.selection.clips.len(), 2);
        e.apply(
            &mut d,
            Action::MoveClips {
                frames: 6000,
                tracks: 1,
            },
        )
        .unwrap();
        assert_eq!(d.audio[0].track, 13);
        assert!(e.undo(&mut d));
        assert_eq!(d.to_json().unwrap(), original);
        assert!(e.redo(&mut d));
        e.select_notes(&d, "m", 0., 4., 0, 127, false).unwrap();
        assert_eq!(e.selection.notes.len(), 2);
        let before = d.to_json().unwrap();
        assert!(e
            .apply(
                &mut d,
                Action::MoveNotes {
                    beats: 0.,
                    semitones: 1
                }
            )
            .is_err());
        assert_eq!(d.to_json().unwrap(), before);
        e.apply(
            &mut d,
            Action::MoveNotes {
                beats: 0.25,
                semitones: 0,
            },
        )
        .unwrap();
        assert_eq!(d.midi[0].notes[0].beat, 0.25);
        assert!(View::from_json(&e.view.to_json().unwrap()).is_ok());
    }
    #[test]
    fn viewport_culls_offscreen_and_reaches_high_track_and_midi_endpoints() {
        let ctx = egui::Context::default();
        let mut d = fixture();
        let mut e = TimelineEditor::default();
        e.view.arrangement_scroll[1] = 12. * 48.;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 700.))),
                ..Default::default()
            },
            |ui| {
                e.arrangement(ui, &mut d, 32).unwrap();
            },
        );
        out.textures_delta.clear();
        assert!(e.last_visible > 0);
        for key in [0, 127] {
            e.view.piano_scroll[1] = (127 - key) as f32 * 16.;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900., 700.))),
                    ..Default::default()
                },
                |ui| {
                    e.piano_roll(ui, &mut d, "m").unwrap();
                },
            );
            out.textures_delta.clear();
            assert!(e.last_visible > 0);
        }
    }
}
#[cfg(test)]
mod keyboard_test {
    use super::*;
    #[test]
    fn focused_region_nudge_is_an_actual_ui_edit() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut d = super::tests::fixture();
        let mut e = TimelineEditor::default();
        e.view.arrangement_scroll[1] = 12. * 48.;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            e.arrangement(ui, &mut d, 32).unwrap();
        });
        out.textures_delta.clear();
        let node = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label().is_some_and(|label| label.starts_with("Clip a,")))
            .unwrap()
            .0;
        let mut focus = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::AccessKitActionRequest(
                    egui::accesskit::ActionRequest {
                        action: egui::accesskit::Action::Focus,
                        target_node: node,
                        target_tree: egui::accesskit::TreeId::ROOT,
                        data: None,
                    },
                )],
                ..Default::default()
            },
            |ui| {
                e.arrangement(ui, &mut d, 32).unwrap();
            },
        );
        focus.textures_delta.clear();
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ui| {
                e.arrangement(ui, &mut d, 32).unwrap();
            },
        );
        out.textures_delta.clear();
        assert_eq!(d.audio[0].start_frame, 54000);
        assert!(e.undo(&mut d));
        assert_eq!(d.audio[0].start_frame, 48000);
    }
}
