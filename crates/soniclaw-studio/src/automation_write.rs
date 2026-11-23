//! Studio automation write takes: isolated Touch/Latch recording with transactional publication.
use crate::{
    project::{Project, ProjectError},
    *,
};
use soniclaw_automation::{Curve, Lane, Mode, Point, Recorder};
fn identity(d: &Document) -> Result<String, ProjectError> {
    let mut d = d.clone();
    d.parameter_values.clear();
    Ok(d.to_json()?)
}
/// Call while stopped, then prepare/publish a new session so its Gesture handles use the selected mode.
pub fn arm(p: &mut Project, parameter: &str, initial: f32, mode: Mode) -> Result<(), ProjectError> {
    if mode == Mode::Read
        || parameter.is_empty()
        || parameter.len() > 256
        || !initial.is_finite()
        || !(0.0..=1.).contains(&initial)
    {
        return Err("invalid automation write arming".into());
    }
    let mut next = p.clone();
    if let Some(l) = next
        .document
        .automation
        .lanes
        .iter_mut()
        .find(|l| l.parameter == parameter)
    {
        l.mode = mode;
        l.enabled = true;
    } else {
        next.document.automation.lanes.push(Lane {
            parameter: parameter.into(),
            enabled: true,
            mode,
            points: vec![Point {
                beat: 0.,
                value: initial,
                curve: Curve::Linear,
            }],
        });
    }
    next.validate()?;
    *p = next;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    project_id: String,
    base: String,
    parameter: String,
    pending: soniclaw_automation::Document,
    grid: f64,
}
pub struct Writer {
    project_id: String,
    base: String,
    parameter: String,
    lane: usize,
    pending: soniclaw_automation::Document,
    recorder: Recorder,
    grid: f64,
}
impl Writer {
    pub fn new(p: &Project, parameter: &str, grid: f64) -> Result<Self, ProjectError> {
        p.validate()?;
        if !grid.is_finite() || !(0.0..=16.).contains(&grid) {
            return Err("automation write grid must be 0..16 beats".into());
        }
        let lane = p
            .document
            .automation
            .lanes
            .iter()
            .position(|l| l.parameter == parameter && l.enabled && l.mode != Mode::Read)
            .ok_or("arm a Touch/Latch lane before preparing playback")?;
        Ok(Self {
            project_id: p.project_id.clone(),
            base: identity(&p.document)?,
            parameter: parameter.into(),
            lane,
            pending: p.document.automation.clone(),
            recorder: Default::default(),
            grid,
        })
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            project_id: self.project_id.clone(),
            base: self.base.clone(),
            parameter: self.parameter.clone(),
            pending: self.pending.clone(),
            grid: self.grid,
        }
    }
    /// Recover only against the unchanged armed project. Recovery never resumes an active gesture.
    pub fn recover(p: &Project, c: Checkpoint) -> Result<Self, ProjectError> {
        if c.project_id != p.project_id || c.base != identity(&p.document)? {
            return Err("automation checkpoint is stale".into());
        }
        c.pending.validate()?;
        let mut writer = Self::new(p, &c.parameter, c.grid)?;
        if c.pending.lanes.len() != writer.pending.lanes.len()
            || c.pending
                .lanes
                .iter()
                .zip(&writer.pending.lanes)
                .any(|(a, b)| a.parameter != b.parameter)
        {
            return Err("automation checkpoint lane mismatch".into());
        }
        writer.pending = c.pending;
        Ok(writer)
    }
    pub fn parameter(&self) -> &str {
        &self.parameter
    }
    pub fn lane_index(&self) -> usize {
        self.lane
    }
    pub fn pending(&self) -> &soniclaw_automation::Document {
        &self.pending
    }
    pub fn point_count(&self) -> usize {
        self.pending.lanes[self.lane].points.len()
    }
    /// Record a GUI/control gesture immediately, including single keyboard/wheel edits. `held` keeps Touch active.
    pub fn input(&mut self, beat: f64, value: f32, held: bool) -> Result<(), ProjectError> {
        self.recorder.begin(value)?;
        let result = self.tick(beat);
        if !held {
            self.recorder.end();
        }
        result.map(|_| ())
    }
    pub fn end(&mut self) {
        self.recorder.end();
    }
    pub fn tick(&mut self, beat: f64) -> Result<bool, ProjectError> {
        Ok(self
            .recorder
            .tick(&mut self.pending, self.lane, beat, self.grid)?)
    }
    pub fn commit(&mut self, p: &mut Project) -> Result<(), ProjectError> {
        self.recorder.stop();
        if self.project_id != p.project_id || identity(&p.document)? != self.base {
            return Err("automation take is stale; project structure or lanes changed".into());
        }
        let mut next = p.clone();
        next.document.automation = self.pending.clone();
        next.validate()?;
        *p = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_latch_snap_and_stale_publication() {
        let (d, a) = crate::tests::fixture();
        let base = Project::new("Write".into(), d, a, Default::default()).unwrap();
        for mode in [Mode::Touch, Mode::Latch] {
            let mut p = base.clone();
            arm(&mut p, "mix.audio.gain", 0.25, mode).unwrap();
            let mut w = Writer::new(&p, "mix.audio.gain", 0.25).unwrap();
            w.input(0.49, 0.8, true).unwrap();
            w.end();
            assert_eq!(w.tick(1.).unwrap(), mode == Mode::Latch);
            p.document
                .parameter_values
                .insert("mix.audio.gain".into(), 0.8);
            w.commit(&mut p).unwrap();
            let lane = &p.document.automation.lanes[0];
            assert!(lane.points.iter().any(|p| p.beat == 0.5 && p.value == 0.8));
            assert_eq!(
                lane.points.iter().any(|p| p.beat == 1.),
                mode == Mode::Latch
            );
            let mut w = Writer::new(&p, "mix.audio.gain", 0.).unwrap();
            w.input(2., 0.1, false).unwrap();
            p.document.tracks[0].input_gain = 0.1;
            let before = p.document.to_json().unwrap();
            assert!(w.commit(&mut p).is_err());
            assert_eq!(p.document.to_json().unwrap(), before);
        }
    }
}
