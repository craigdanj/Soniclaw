//! Control-thread beat documents and bounded, allocation-free prepared playback.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::{
    atomic::{AtomicU32, AtomicU8, Ordering},
    Arc,
};
#[cfg(feature = "ui")]
pub mod ui;
pub const MAX_LANES: usize = 64;
pub const MAX_POINTS: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    Step,
    #[default]
    Linear,
    Smooth,
    Bezier {
        c1: f32,
        c2: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Read,
    Touch,
    Latch,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub beat: f64,
    pub value: f32,
    pub curve: Curve,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub parameter: String,
    pub enabled: bool,
    pub mode: Mode,
    pub points: Vec<Point>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub lanes: Vec<Lane>,
    pub loop_range: Option<[f64; 2]>,
}
fn beat_ok(b: f64) -> bool {
    b.is_finite() && (0.0..=1_000_000.0).contains(&b)
}
fn value_ok(v: f32) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}
impl Document {
    pub fn validate(&self) -> Result<(), Error> {
        if self.lanes.len() > MAX_LANES
            || self.lanes.iter().map(|l| l.points.len()).sum::<usize>() > MAX_POINTS
        {
            return Err(Error::InvalidParameter);
        }
        if self
            .loop_range
            .is_some_and(|[a, b]| !beat_ok(a) || !beat_ok(b) || a >= b)
        {
            return Err(Error::InvalidParameter);
        }
        for (i, l) in self.lanes.iter().enumerate() {
            if l.parameter.is_empty()
                || l.parameter.len() > 256
                || self.lanes[..i].iter().any(|p| p.parameter == l.parameter)
            {
                return Err(Error::InvalidParameter);
            }
            let mut last = None;
            for p in &l.points {
                if !beat_ok(p.beat) || !value_ok(p.value) || last.is_some_and(|b| p.beat <= b) {
                    return Err(Error::InvalidParameter);
                }
                if let Curve::Bezier { c1, c2 } = p.curve {
                    if !value_ok(c1) || !value_ok(c2) {
                        return Err(Error::InvalidParameter);
                    }
                }
                last = Some(p.beat);
            }
        }
        Ok(())
    }
    pub fn beat(&self, b: f64) -> f64 {
        self.loop_range.map_or(b, |[a, z]| {
            if b >= a {
                a + (b - a).rem_euclid(z - a)
            } else {
                b
            }
        })
    }
    /// Transactional point insertion/replacement. Allocate only on the control thread.
    pub fn write(&mut self, lane: usize, beat: f64, value: f32, grid: f64) -> Result<(), Error> {
        self.validate()?;
        let b = snap(beat, grid)?;
        if !value_ok(value) || lane >= self.lanes.len() {
            return Err(Error::InvalidParameter);
        }
        let points = &self.lanes[lane].points;
        let at = points.partition_point(|p| p.beat < b);
        let replace = points.get(at).is_some_and(|p| p.beat == b);
        if !replace && self.lanes.iter().map(|l| l.points.len()).sum::<usize>() == MAX_POINTS {
            return Err(Error::InvalidParameter);
        }
        let p = Point {
            beat: b,
            value,
            curve: Curve::Linear,
        };
        if replace {
            self.lanes[lane].points[at] = p;
        } else {
            self.lanes[lane].points.insert(at, p);
        }
        Ok(())
    }
}
pub fn snap(beat: f64, grid: f64) -> Result<f64, Error> {
    if !beat_ok(beat) || !grid.is_finite() || grid < 0.0 {
        return Err(Error::InvalidParameter);
    }
    let b = if grid > 0.0 {
        (beat / grid).round() * grid
    } else {
        beat
    };
    if !beat_ok(b) {
        return Err(Error::InvalidParameter);
    }
    Ok(b)
}
impl Lane {
    pub fn value_at(&self, beat: f64) -> Option<f32> {
        if !self.enabled || self.points.is_empty() || !beat.is_finite() {
            return None;
        }
        let i = self.points.partition_point(|p| p.beat <= beat);
        if i == 0 {
            return Some(self.points[0].value);
        }
        let a = &self.points[i - 1];
        let Some(b) = self.points.get(i) else {
            return Some(a.value);
        };
        let t = ((beat - a.beat) / (b.beat - a.beat)) as f32;
        let u = match a.curve {
            Curve::Step => 0.0,
            Curve::Linear => t,
            Curve::Smooth => t * t * (3.0 - 2.0 * t),
            Curve::Bezier { c1, c2 } => {
                3.0 * (1.0 - t).powi(2) * t * c1 + 3.0 * (1.0 - t) * t * t * c2 + t * t * t
            }
        };
        Some(a.value + (b.value - a.value) * u)
    }
}
#[derive(Default)]
struct GestureState {
    value: AtomicU32,
    active: AtomicU8,
}
/// A cloned UI/control handle. End Touch restores the lane; end Latch holds until stop/reset.
#[derive(Clone)]
pub struct Gesture {
    mode: Mode,
    state: Arc<GestureState>,
}
impl Gesture {
    pub fn set(&self, value: f32) -> Result<(), Error> {
        if !value_ok(value) {
            return Err(Error::InvalidParameter);
        }
        self.state.value.store(value.to_bits(), Ordering::Relaxed);
        if self.mode != Mode::Read {
            self.state.active.store(1, Ordering::Release);
        }
        Ok(())
    }
    pub fn end(&self) {
        if self.mode == Mode::Touch {
            self.state.active.store(0, Ordering::Release);
        }
    }
    pub fn clear(&self) {
        self.state.active.store(0, Ordering::Release);
    }
    fn value(&self) -> Option<f32> {
        (self.state.active.load(Ordering::Acquire) != 0)
            .then(|| f32::from_bits(self.state.value.load(Ordering::Relaxed)))
    }
}
/// Control-thread recorder. Feed exact transport beats, or the UI's coarser cursor.
#[derive(Default)]
pub struct Recorder {
    held: bool,
    latched: bool,
    value: f32,
}
impl Recorder {
    pub fn begin(&mut self, value: f32) -> Result<(), Error> {
        if !value_ok(value) {
            return Err(Error::InvalidParameter);
        }
        self.held = true;
        self.value = value;
        Ok(())
    }
    pub fn end(&mut self) {
        self.held = false;
    }
    pub fn stop(&mut self) {
        self.held = false;
        self.latched = false;
    }
    pub fn tick(
        &mut self,
        doc: &mut Document,
        lane: usize,
        beat: f64,
        grid: f64,
    ) -> Result<bool, Error> {
        let mode = doc.lanes.get(lane).ok_or(Error::InvalidParameter)?.mode;
        if mode == Mode::Latch && self.held {
            self.latched = true;
        }
        if mode != Mode::Read && (self.held || mode == Mode::Latch && self.latched) {
            doc.write(lane, beat, self.value, grid)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
/// Prepared wrapper. Beat continuity is preserved through contiguous tempo changes;
/// explicit seeks re-anchor from absolute sample position using the current tempo.
pub struct AutomatedProcessor {
    tempo_map: Option<Arc<soniclaw_transport::TempoMap>>,
    inner: Box<dyn AudioProcessor>,
    document: Document,
    targets: Vec<Arc<Parameter>>,
    gestures: Vec<Gesture>,
    spec: Option<ProcessSpec>,
    end: Option<u64>,
    beat: f64,
    scratch: Vec<Event>,
}
impl AutomatedProcessor {
    pub fn with_tempo_map(mut self, map: Arc<soniclaw_transport::TempoMap>) -> Self {
        self.tempo_map = Some(map);
        self
    }

    pub fn new(
        inner: Box<dyn AudioProcessor>,
        document: Document,
        parameters: &[Arc<Parameter>],
    ) -> Result<(Self, Vec<Gesture>), Error> {
        document.validate()?;
        let mut targets = vec![];
        let mut gestures = vec![];
        for lane in &document.lanes {
            targets.push(
                parameters
                    .iter()
                    .find(|p| p.info().id == lane.parameter)
                    .ok_or(Error::InvalidParameter)?
                    .clone(),
            );
            gestures.push(Gesture {
                mode: lane.mode,
                state: Arc::new(Default::default()),
            });
        }
        let handles = gestures.clone();
        Ok((
            Self {
                inner,
                tempo_map: None,
                document,
                targets,
                gestures,
                spec: None,
                end: None,
                beat: 0.0,
                scratch: vec![],
            },
            handles,
        ))
    }
}
impl AudioProcessor for AutomatedProcessor {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        self.inner.prepare(spec)?;
        self.scratch = vec![
            Event {
                frame: 0,
                message: MidiMessage::AllNotesOff
            };
            1024
        ];
        self.spec = Some(spec);
        self.end = None;
        Ok(())
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.end = None;
        for g in &self.gestures {
            g.clear();
        }
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            b.clear();
            return;
        };
        if ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
            || b.channels() != spec.channels
            || b.frames() > spec.max_block_frames
            || !ctx.transport.tempo.is_finite()
            || ctx.transport.tempo <= 0.0
        {
            b.clear();
            return;
        }
        if !ctx.transport.playing {
            for g in &self.gestures {
                g.clear();
            }
            self.end = None;
            self.inner.process(b, ctx);
            return;
        }
        if self
            .document
            .lanes
            .iter()
            .all(|l| !l.enabled || l.points.is_empty())
        {
            self.inner.process(b, ctx);
            return;
        }
        let step = ctx.transport.tempo / (60.0 * spec.sample_rate);
        if self.end != Some(ctx.transport.sample_position) {
            self.beat = ctx.transport.sample_position as f64 * step;
        }
        let start_beat = self.beat;
        let frames = b.frames();
        let mut mi = 0;
        for (frame, samples) in b.samples_mut().chunks_exact_mut(spec.channels).enumerate() {
            let timeline_beat = if let Some(map) = &self.tempo_map {
                match map.beat_at_sample(
                    ctx.transport.sample_position.saturating_add(frame as u64),
                    spec.sample_rate,
                ) {
                    Ok(b) => b,
                    Err(_) => {
                        samples.fill(0.0);
                        continue;
                    }
                }
            } else {
                start_beat + frame as f64 * step
            };
            let beat = self.document.beat(timeline_beat);
            for ((lane, p), g) in self
                .document
                .lanes
                .iter()
                .zip(&self.targets)
                .zip(&self.gestures)
            {
                if lane.enabled {
                    if let Some(v) = g.value().or_else(|| lane.value_at(beat)) {
                        let info = p.info();
                        let _ = p.set(info.min + v * (info.max - info.min));
                    }
                }
            }
            let mut count = 0;
            while mi < ctx.events.len() && ctx.events[mi].frame == frame {
                self.scratch[count] = Event {
                    frame: 0,
                    message: ctx.events[mi].message,
                };
                count += 1;
                mi += 1;
            }
            let mut one = AudioBuffer::new(samples, spec.channels).expect("prepared channels");
            self.inner.process(
                &mut one,
                &ProcessContext {
                    events: &self.scratch[..count],
                    transport: Transport {
                        sample_position: ctx.transport.sample_position.saturating_add(frame as u64),
                        ..ctx.transport
                    },
                },
            );
        }
        self.beat = start_beat + frames as f64 * step;
        self.end = Some(ctx.transport.sample_position.saturating_add(frames as u64));
    }
}
