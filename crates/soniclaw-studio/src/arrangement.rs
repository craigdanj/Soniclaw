//! Musical timeline edits, song labels and nondestructive arrangement duplication. Control thread only.
use crate::{
    project::{Project, ProjectError},
    workspace, *,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub id: String,
    pub name: String,
    pub beat: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub markers: Vec<Marker>,
    pub sections: Vec<Section>,
}
fn beat(v: f64) -> bool {
    v.is_finite() && (0.0..=1_000_000.).contains(&v)
}
impl Model {
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty() && self.sections.is_empty()
    }
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.markers.len() > 1024 || self.sections.len() > 128 {
            return Err("song label count limit".into());
        }
        let mut ids = BTreeSet::new();
        for (id, name, at, end) in self
            .markers
            .iter()
            .map(|m| (&m.id, &m.name, m.beat, None))
            .chain(
                self.sections
                    .iter()
                    .map(|s| (&s.id, &s.name, s.start, Some(s.end))),
            )
        {
            if !valid_id(id)
                || !ids.insert(id)
                || name.trim().is_empty()
                || name.len() > 256
                || !beat(at)
                || end.is_some_and(|end| !beat(end) || end <= at)
            {
                return Err("invalid song marker/section".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum AudioAnchor {
    Seconds,
    Beats,
}
#[derive(Clone, Debug)]
pub enum Command {
    Tempo(soniclaw_transport::TempoPoint),
    RemoveTempo(f64),
    Meter(soniclaw_transport::MeterPoint),
    RemoveMeter(f64),
    Marker(Marker),
    RemoveMarker(String),
    Section(Section),
    RemoveSection(String),
}
pub fn change_timeline(
    p: &mut Project,
    timeline: Timeline,
    anchor: AudioAnchor,
) -> Result<(), ProjectError> {
    TempoMap::compile(&timeline)?;
    if timeline == p.document.timeline {
        return Ok(());
    }
    if !p.warps.is_empty() || !p.freezes.is_empty() {
        return Err("unfreeze/rebuild tempo caches before changing their timeline".into());
    }
    if matches!(anchor, AudioAnchor::Beats) && !p.comps.is_empty() {
        return Err(
            "comp recipes use fixed sample positions; choose audio seconds before changing tempo"
                .into(),
        );
    }
    let mut next = p.clone();
    if matches!(anchor, AudioAnchor::Beats) {
        let old = TempoMap::compile(&p.document.timeline)?;
        let new = TempoMap::compile(&timeline)?;
        for t in &mut next.document.tracks {
            for c in &mut t.audio {
                c.start_frame = new.sample_at(
                    old.beat_at_sample(c.start_frame, p.document.sample_rate as f64)?,
                    p.document.sample_rate as f64,
                )?;
            }
            for c in &mut t.streamed {
                c.start_frame = new.sample_at(
                    old.beat_at_sample(c.start_frame, p.document.sample_rate as f64)?,
                    p.document.sample_rate as f64,
                )?;
            }
        }
    }
    next.document.timeline = timeline;
    next.validate()?;
    *p = next;
    Ok(())
}
pub fn apply(p: &mut Project, command: Command, anchor: AudioAnchor) -> Result<(), ProjectError> {
    let mut next = p.clone();
    let mut timeline = next.document.timeline.clone();
    let mut timing = false;
    match command {
        Command::Tempo(point) => {
            timeline.tempo.retain(|p| p.beat != point.beat);
            timeline.tempo.push(point);
            timeline.tempo.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            timing = true;
        }
        Command::RemoveTempo(at) => {
            if at == 0. || !timeline.tempo.iter().any(|p| p.beat == at) {
                return Err("cannot remove initial/missing tempo".into());
            }
            timeline.tempo.retain(|p| p.beat != at);
            timing = true;
        }
        Command::Meter(point) => {
            timeline.meters.retain(|p| p.beat != point.beat);
            timeline.meters.push(point);
            timeline.meters.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            timing = true;
        }
        Command::RemoveMeter(at) => {
            if at == 0. || !timeline.meters.iter().any(|p| p.beat == at) {
                return Err("cannot remove initial/missing meter".into());
            }
            timeline.meters.retain(|p| p.beat != at);
            timing = true;
        }
        Command::Marker(m) => {
            next.layout.arrangement.markers.retain(|old| old.id != m.id);
            next.layout.arrangement.markers.push(m);
        }
        Command::RemoveMarker(id) => {
            if !next.layout.arrangement.markers.iter().any(|m| m.id == id) {
                return Err("marker missing".into());
            }
            next.layout.arrangement.markers.retain(|m| m.id != id);
        }
        Command::Section(s) => {
            next.layout
                .arrangement
                .sections
                .retain(|old| old.id != s.id);
            next.layout.arrangement.sections.push(s);
        }
        Command::RemoveSection(id) => {
            if !next.layout.arrangement.sections.iter().any(|s| s.id == id) {
                return Err("section missing".into());
            }
            next.layout.arrangement.sections.retain(|s| s.id != id);
        }
    }
    if timing {
        change_timeline(&mut next, timeline, anchor)?;
    }
    next.layout
        .arrangement
        .markers
        .sort_by(|a, b| a.beat.total_cmp(&b.beat));
    next.layout
        .arrangement
        .sections
        .sort_by(|a, b| a.start.total_cmp(&b.start));
    next.validate()?;
    *p = next;
    Ok(())
}
fn copy_region(
    c: &soniclaw_clips::AudioClip,
    a: u64,
    b: u64,
) -> Result<soniclaw_clips::AudioClip, ProjectError> {
    let mut c = c.clone();
    let shift = a - c.start_frame;
    let m = c
        .edits
        .get_or_insert_with(|| Box::new(soniclaw_clips::RegionEdits::new(c.length_frames)));
    m.source_phase = m
        .source_phase
        .checked_add(shift)
        .ok_or("audio phase overflow")?;
    m.fade_offset = m
        .fade_offset
        .checked_add(shift)
        .ok_or("fade phase overflow")?;
    m.envelope_offset = m
        .envelope_offset
        .checked_add(shift)
        .ok_or("envelope phase overflow")?;
    c.start_frame = a;
    c.length_frames = b - a;
    Ok(c)
}
fn curve_part(
    curve: soniclaw_automation::Curve,
    t0: f64,
    t1: f64,
) -> Result<soniclaw_automation::Curve, ProjectError> {
    use soniclaw_automation::Curve;
    if matches!(curve, Curve::Step | Curve::Linear) {
        return Ok(curve);
    }
    let (c1, c2) = match curve {
        Curve::Smooth => (0., 1.),
        Curve::Bezier { c1, c2 } => (c1 as f64, c2 as f64),
        _ => unreachable!(),
    };
    let f = |t: f64| 3. * (1. - t).powi(2) * t * c1 + 3. * (1. - t) * t * t * c2 + t * t * t;
    let d =
        |t: f64| 3. * ((1. - t).powi(2) * c1 + 2. * (1. - t) * t * (c2 - c1) + t * t * (1. - c2));
    let delta = f(t1) - f(t0);
    if delta.abs() < 1e-12 {
        return Ok(Curve::Linear);
    }
    let a = (t1 - t0) * d(t0) / (3. * delta);
    let b = 1. - (t1 - t0) * d(t1) / (3. * delta);
    if !(-1e-5..=1.00001).contains(&a) || !(-1e-5..=1.00001).contains(&b) {
        return Err("automation subcurve exceeds representation".into());
    }
    Ok(Curve::Bezier {
        c1: a.clamp(0., 1.) as f32,
        c2: b.clamp(0., 1.) as f32,
    })
}
fn copy_automation(
    p: &mut Project,
    a: f64,
    b: f64,
    destination: f64,
    tracks: Option<&BTreeSet<usize>>,
) -> Result<(), ProjectError> {
    use soniclaw_automation::{Curve, Point};
    let prefixes = tracks.map(|tracks| {
        tracks
            .iter()
            .flat_map(|&i| {
                let t = &p.document.tracks[i];
                std::iter::once(format!("mix.{}.", t.id))
                    .chain(
                        t.instrument
                            .iter()
                            .chain(&t.inserts)
                            .map(|d| format!("{}.", d.id)),
                    )
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    });
    for lane in &mut p.document.automation.lanes {
        if prefixes.as_ref().is_some_and(|prefixes| {
            !prefixes
                .iter()
                .any(|prefix| lane.parameter.starts_with(prefix))
        }) {
            continue;
        }
        if lane.points.is_empty() {
            continue;
        }
        let mut source = lane.clone();
        source.enabled = true;
        let mut knots = vec![a];
        knots.extend(
            source
                .points
                .iter()
                .filter(|p| p.beat > a && p.beat < b)
                .map(|p| p.beat),
        );
        knots.push(b);
        let mut copied = vec![];
        for (i, &at) in knots.iter().enumerate() {
            let index = source.points.partition_point(|p| p.beat <= at);
            let curve = if i + 1 == knots.len() || index == 0 || index == source.points.len() {
                Curve::Step
            } else {
                let left = &source.points[index - 1];
                let right = &source.points[index];
                curve_part(
                    left.curve,
                    (at - left.beat) / (right.beat - left.beat),
                    (knots[i + 1] - left.beat) / (right.beat - left.beat),
                )?
            };
            copied.push(Point {
                beat: destination + at - a,
                value: source.value_at(at).ok_or("automation value unavailable")?,
                curve,
            });
        }
        let end = destination + b - a;
        lane.points.retain(|p| p.beat < destination || p.beat > end);
        lane.points.extend(copied);
        lane.points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
    Ok(())
}
fn duplicate(
    p: &mut Project,
    a: f64,
    b: f64,
    destination: f64,
    ids: Option<&BTreeSet<String>>,
    automation: bool,
) -> Result<BTreeSet<String>, ProjectError> {
    if !beat(a) || !beat(b) || a >= b || !beat(destination) || !beat(destination + b - a) {
        return Err("invalid arrangement copy range".into());
    }
    let mut next = p.clone();
    let mut view = next.edit_view()?;
    let map = TempoMap::compile(&view.document.timeline)?;
    let rate = p.document.sample_rate as f64;
    let start = map.sample_at(a, rate)?;
    let end = map.sample_at(b, rate)?;
    let mut added = BTreeSet::new();
    let mut tracks = BTreeSet::new();
    let mut links = BTreeMap::<String, String>::new();
    for c in view.document.audio.clone() {
        if ids.is_some_and(|ids| !ids.contains(&c.id)) {
            continue;
        }
        let x = c.start_frame.max(start);
        let y = (c.start_frame + c.length_frames).min(end);
        if x >= y {
            continue;
        }
        let mut copied = copy_region(&c, x, y)?;
        copied.id = workspace::identity("clip")?;
        copied.start_frame = map.sample_at(destination + map.beat_at_sample(x, rate)? - a, rate)?;
        if let Some(m) = &mut copied.edits {
            if let Some(old) = m.link_group.clone() {
                let new = if let Some(id) = links.get(&old) {
                    id.clone()
                } else {
                    let id = workspace::identity("link")?;
                    links.insert(old, id.clone());
                    id
                };
                m.link_group = Some(new);
            }
        }
        tracks.insert(c.track as usize);
        added.insert(copied.id.clone());
        view.document.audio.push(copied);
    }
    for c in view.document.midi.clone() {
        if ids.is_some_and(|ids| !ids.contains(&c.id)) {
            continue;
        }
        let x = c.start_beat.max(a);
        let y = (c.start_beat + c.length_beats).min(b);
        if x >= y {
            continue;
        }
        let mut copied = c.clone();
        copied.id = workspace::identity("clip")?;
        copied.start_beat = destination + x - a;
        copied.length_beats = y - x;
        let phase = c.source_offset + x - c.start_beat;
        if c.looping {
            copied.source_offset = phase % c.source_length;
        } else if phase < c.source_length {
            copied.source_offset = phase;
        } else {
            copied.source_offset = 0.;
            copied.source_length = y - x;
            copied.notes.clear();
            copied.controls.clear();
        }
        tracks.insert(c.track as usize);
        added.insert(copied.id.clone());
        view.document.midi.push(copied);
    }
    if added.is_empty() {
        return Err("copy range contains no selected regions".into());
    }
    next.replace_edit_view(view)?;
    if automation {
        copy_automation(
            &mut next,
            a,
            b,
            destination,
            if ids.is_some() { Some(&tracks) } else { None },
        )?;
    }
    next.validate()?;
    *p = next;
    Ok(added)
}
pub fn duplicate_selection(
    p: &mut Project,
    ids: &BTreeSet<String>,
    destination: f64,
    automation: bool,
) -> Result<BTreeSet<String>, ProjectError> {
    if ids.is_empty() {
        return Err("select arrangement regions first".into());
    }
    let view = p.edit_view()?;
    let map = TempoMap::compile(&view.document.timeline)?;
    let rate = p.document.sample_rate as f64;
    let mut a = f64::INFINITY;
    let mut b = 0f64;
    let mut found = BTreeSet::new();
    for c in &view.document.audio {
        if ids.contains(&c.id) {
            found.insert(c.id.clone());
            a = a.min(map.beat_at_sample(c.start_frame, rate)?);
            b = b.max(map.beat_at_sample(c.start_frame + c.length_frames, rate)?);
        }
    }
    for c in &view.document.midi {
        if ids.contains(&c.id) {
            found.insert(c.id.clone());
            a = a.min(c.start_beat);
            b = b.max(c.start_beat + c.length_beats);
        }
    }
    if &found != ids {
        return Err("selection contains missing regions".into());
    }
    duplicate(p, a, b, destination, Some(ids), automation)
}
pub fn duplicate_section(
    p: &mut Project,
    id: &str,
    destination: f64,
    automation: bool,
) -> Result<BTreeSet<String>, ProjectError> {
    let section = p
        .layout
        .arrangement
        .sections
        .iter()
        .find(|s| s.id == id)
        .ok_or("section missing")?
        .clone();
    let mut next = p.clone();
    let added = duplicate(
        &mut next,
        section.start,
        section.end,
        destination,
        None,
        automation,
    )?;
    let label = Section {
        id: workspace::identity("section")?,
        name: format!("{} copy", section.name),
        start: destination,
        end: destination + section.end - section.start,
    };
    apply(&mut next, Command::Section(label), AudioAnchor::Seconds)?;
    *p = next;
    Ok(added)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_timing_or_labels_leave_project_unchanged() {
        let mut p = workspace::blank("Labels".into(), 48000, 128).unwrap();
        let original = p.clone();
        assert!(apply(
            &mut p,
            Command::Tempo(soniclaw_transport::TempoPoint {
                beat: 0.,
                bpm: f64::NAN,
                transition: soniclaw_transport::Transition::Step
            }),
            AudioAnchor::Seconds
        )
        .is_err());
        assert!(apply(&mut p, Command::RemoveMeter(0.), AudioAnchor::Seconds).is_err());
        assert!(apply(
            &mut p,
            Command::Marker(Marker {
                id: "bad".into(),
                name: "".into(),
                beat: 1.
            }),
            AudioAnchor::Seconds
        )
        .is_err());
        assert!(history::equivalent(&p, &original).unwrap());
    }
    #[test]
    fn partial_automation_splines_keep_values() {
        for curve in [
            soniclaw_automation::Curve::Smooth,
            soniclaw_automation::Curve::Bezier { c1: 0.8, c2: 0.2 },
        ] {
            let original = soniclaw_automation::Lane {
                parameter: "test".into(),
                enabled: true,
                mode: soniclaw_automation::Mode::Read,
                points: vec![
                    soniclaw_automation::Point {
                        beat: 0.,
                        value: 0.1,
                        curve,
                    },
                    soniclaw_automation::Point {
                        beat: 4.,
                        value: 0.9,
                        curve: soniclaw_automation::Curve::Step,
                    },
                ],
            };
            let mut p = workspace::blank("Automation".into(), 48000, 128).unwrap();
            p.document.automation.lanes.push(original.clone());
            copy_automation(&mut p, 0.7, 3.4, 10., None).unwrap();
            for i in 0..100 {
                let x = i as f64 / 100. * 2.7;
                assert!(
                    (p.document.automation.lanes[0].value_at(10. + x).unwrap()
                        - original.value_at(0.7 + x).unwrap())
                    .abs()
                        < 1e-5
                );
            }
        }
    }
}
