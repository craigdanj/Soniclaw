//! Prepared musical clocks. Beats are quarter notes (PPQ); tempo ramps are linear in beat space.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
const MAX_BEAT: f64 = 1_000_000.0;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Transition {
    Step,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoPoint {
    pub beat: f64,
    pub bpm: f64,
    pub transition: Transition,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPoint {
    pub beat: f64,
    pub numerator: u16,
    pub denominator: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub schema: u32,
    pub tempo: Vec<TempoPoint>,
    pub meters: Vec<MeterPoint>,
    pub loop_range: Option<[f64; 2]>,
    pub punch_range: Option<[f64; 2]>,
    pub panic_on_loop: bool,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            schema: 1,
            tempo: vec![TempoPoint {
                beat: 0.0,
                bpm: 120.0,
                transition: Transition::Step,
            }],
            meters: vec![MeterPoint {
                beat: 0.0,
                numerator: 4,
                denominator: 4,
            }],
            loop_range: None,
            punch_range: None,
            panic_on_loop: true,
        }
    }
}
fn beat_ok(b: f64) -> bool {
    b.is_finite() && (0.0..=MAX_BEAT).contains(&b)
}
#[derive(Clone, PartialEq)]
struct Segment {
    beat: f64,
    bpm: f64,
    slope: f64,
    seconds: f64,
}
#[derive(Clone, PartialEq)]
struct Meter {
    point: MeterPoint,
    bar: u64,
}
#[derive(Clone, PartialEq)]
pub struct TempoMap {
    segments: Vec<Segment>,
    meters: Vec<Meter>,
}
impl TempoMap {
    pub fn compile(timeline: &Timeline) -> Result<Self, Error> {
        if timeline.schema != 1
            || timeline.tempo.is_empty()
            || timeline.tempo.len() > 1024
            || timeline.meters.is_empty()
            || timeline.meters.len() > 1024
            || timeline.tempo[0].beat != 0.0
            || timeline.meters[0].beat != 0.0
        {
            return Err(Error::InvalidParameter);
        }
        for range in [timeline.loop_range, timeline.punch_range]
            .into_iter()
            .flatten()
        {
            if !beat_ok(range[0]) || !beat_ok(range[1]) || range[0] >= range[1] {
                return Err(Error::InvalidParameter);
            }
        }
        let mut segments = vec![];
        let mut seconds = 0.0;
        for (i, p) in timeline.tempo.iter().enumerate() {
            if !beat_ok(p.beat)
                || !p.bpm.is_finite()
                || !(20.0..=400.0).contains(&p.bpm)
                || i > 0 && p.beat <= timeline.tempo[i - 1].beat
            {
                return Err(Error::InvalidParameter);
            }
            let slope = if let Some(next) = timeline.tempo.get(i + 1) {
                if !beat_ok(next.beat) || next.beat <= p.beat {
                    return Err(Error::InvalidParameter);
                }
                if p.transition == Transition::Linear {
                    (next.bpm - p.bpm) / (next.beat - p.beat)
                } else {
                    0.0
                }
            } else {
                0.0
            };
            segments.push(Segment {
                beat: p.beat,
                bpm: p.bpm,
                slope,
                seconds,
            });
            if let Some(next) = timeline.tempo.get(i + 1) {
                seconds += integral(p.bpm, slope, next.beat - p.beat);
                if !seconds.is_finite() {
                    return Err(Error::InvalidParameter);
                }
            }
        }
        let mut meters: Vec<Meter> = vec![];
        for (i, p) in timeline.meters.iter().enumerate() {
            if !beat_ok(p.beat)
                || p.numerator == 0
                || p.numerator > 32
                || !p.denominator.is_power_of_two()
                || p.denominator > 64
                || i > 0 && p.beat <= timeline.meters[i - 1].beat
            {
                return Err(Error::InvalidParameter);
            }
            let bar = if let Some(previous) = meters.last() {
                let length =
                    previous.point.numerator as f64 * 4.0 / previous.point.denominator as f64;
                let bars = (p.beat - previous.point.beat) / length;
                if (bars - bars.round()).abs() > 1e-7 {
                    return Err(Error::InvalidParameter);
                }
                previous.bar + bars.round() as u64
            } else {
                0
            };
            meters.push(Meter { point: *p, bar });
        }
        Ok(Self { segments, meters })
    }
    pub fn seconds_at(&self, beat: f64) -> Result<f64, Error> {
        if !beat_ok(beat) {
            return Err(Error::InvalidParameter);
        }
        let index = self.segments.partition_point(|s| s.beat <= beat) - 1;
        let s = &self.segments[index];
        Ok(s.seconds + integral(s.bpm, s.slope, beat - s.beat))
    }
    pub fn beat_at_seconds(&self, seconds: f64) -> Result<f64, Error> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(Error::InvalidParameter);
        }
        let index = self.segments.partition_point(|s| s.seconds <= seconds) - 1;
        let s = &self.segments[index];
        let delta = if s.slope.abs() < 1e-12 {
            (seconds - s.seconds) * s.bpm / 60.0
        } else {
            s.bpm * ((seconds - s.seconds) * s.slope / 60.0).exp_m1() / s.slope
        };
        let beat = s.beat + delta;
        if !beat_ok(beat) {
            return Err(Error::InvalidParameter);
        }
        Ok(beat)
    }
    pub fn sample_at(&self, beat: f64, rate: f64) -> Result<u64, Error> {
        valid_rate(rate)?;
        let sample = (self.seconds_at(beat)? * rate).round();
        if sample > u64::MAX as f64 {
            return Err(Error::InvalidParameter);
        }
        Ok(sample as u64)
    }
    pub fn beat_at_sample(&self, sample: u64, rate: f64) -> Result<f64, Error> {
        valid_rate(rate)?;
        self.beat_at_seconds(sample as f64 / rate)
    }
    pub fn position(&self, beat: f64) -> Result<Position, Error> {
        if !beat_ok(beat) {
            return Err(Error::InvalidParameter);
        }
        let s = &self.segments[self.segments.partition_point(|s| s.beat <= beat) - 1];
        let m = &self.meters[self.meters.partition_point(|m| m.point.beat <= beat) - 1];
        let length = m.point.numerator as f64 * 4.0 / m.point.denominator as f64;
        let delta = beat - m.point.beat;
        let bars = (delta / length).floor();
        Ok(Position {
            beat,
            bpm: s.bpm + s.slope * (beat - s.beat),
            bar: m.bar + bars as u64,
            beat_in_bar: delta - bars * length,
            numerator: m.point.numerator,
            denominator: m.point.denominator,
        })
    }
}
fn valid_rate(rate: f64) -> Result<(), Error> {
    if !rate.is_finite() || !(1.0..=768000.0).contains(&rate) {
        Err(Error::InvalidSpec)
    } else {
        Ok(())
    }
}
fn integral(bpm: f64, slope: f64, beats: f64) -> f64 {
    if slope.abs() < 1e-12 {
        beats * 60.0 / bpm
    } else {
        60.0 / slope * (slope * beats / bpm).ln_1p()
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Position {
    pub beat: f64,
    pub bpm: f64,
    pub bar: u64,
    pub beat_in_bar: f64,
    pub numerator: u16,
    pub denominator: u16,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Frame {
    pub sample: u64,
    pub position: Position,
    pub playing: bool,
    pub punching: bool,
    pub looped: bool,
    pub loop_range: Option<[f64; 2]>,
    pub loop_seconds: Option<[f64; 2]>,
}
impl Frame {
    pub fn core_transport(&self) -> Transport {
        Transport {
            sample_position: self.sample,
            tempo: self.position.bpm,
            playing: self.playing,
        }
    }
}
pub struct Clock {
    map: Arc<TempoMap>,
    rate: f64,
    sample: u64,
    playing: bool,
    loop_samples: Option<[u64; 2]>,
    punch_samples: Option<[u64; 2]>,
    looped: bool,
    loop_beats: Option<[f64; 2]>,
    loop_seconds: Option<[f64; 2]>,
}
impl Clock {
    pub fn new(timeline: &Timeline, rate: f64) -> Result<Self, Error> {
        Self::from_map(timeline, rate, Arc::new(TempoMap::compile(timeline)?))
    }
    pub fn from_map(timeline: &Timeline, rate: f64, map: Arc<TempoMap>) -> Result<Self, Error> {
        valid_rate(rate)?;
        if TempoMap::compile(timeline)? != *map {
            return Err(Error::InvalidParameter);
        }
        let range = |r: Option<[f64; 2]>| -> Result<Option<[u64; 2]>, Error> {
            if let Some([a, b]) = r {
                let range = [map.sample_at(a, rate)?, map.sample_at(b, rate)?];
                if range[0] >= range[1] {
                    return Err(Error::InvalidParameter);
                }
                Ok(Some(range))
            } else {
                Ok(None)
            }
        };
        Ok(Self {
            loop_samples: range(timeline.loop_range)?,
            punch_samples: range(timeline.punch_range)?,
            map: map.clone(),
            rate,
            sample: 0,
            playing: false,
            looped: false,
            loop_beats: timeline.loop_range,
            loop_seconds: timeline
                .loop_range
                .map(|[a, b]| Ok::<_, Error>([map.seconds_at(a)?, map.seconds_at(b)?]))
                .transpose()?,
        })
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }
    pub fn seek(&mut self, sample: u64) -> Result<(), Error> {
        self.map.beat_at_sample(sample, self.rate)?;
        self.sample = if let Some([a, b]) = self.loop_samples {
            if sample >= b {
                a + (sample - a) % (b - a)
            } else {
                sample
            }
        } else {
            sample
        };
        self.looped = false;
        Ok(())
    }
    pub fn frame(&self) -> Result<Frame, Error> {
        let beat = self.map.beat_at_sample(self.sample, self.rate)?;
        Ok(Frame {
            sample: self.sample,
            position: self.map.position(beat)?,
            playing: self.playing,
            punching: self.playing
                && self
                    .punch_samples
                    .is_none_or(|[a, b]| (a..b).contains(&self.sample)),
            looped: self.looped,
            loop_range: self.loop_beats,
            loop_seconds: self.loop_seconds,
        })
    }
    pub fn advance(&mut self, frames: usize) -> Result<(), Error> {
        if frames > 8192 {
            return Err(Error::BlockTooLarge);
        }
        self.looped = false;
        if !self.playing {
            return Ok(());
        }
        let next = self
            .sample
            .checked_add(frames as u64)
            .ok_or(Error::InvalidParameter)?;
        self.sample = if let Some([a, b]) = self.loop_samples {
            if next >= b {
                self.looped = true;
                a + (next - a) % (b - a)
            } else {
                next
            }
        } else {
            self.map.beat_at_sample(next, self.rate)?;
            next
        };
        Ok(())
    }
}
pub struct TimelineProcessor {
    inner: Box<dyn AudioProcessor>,
    timeline: Timeline,
    map: Arc<TempoMap>,
    clock: Option<Clock>,
    spec: Option<ProcessSpec>,
    scratch: Vec<Event>,
    source_end: Option<u64>,
}
impl TimelineProcessor {
    pub fn new(inner: Box<dyn AudioProcessor>, timeline: Timeline) -> Result<Self, Error> {
        let map = Arc::new(TempoMap::compile(&timeline)?);
        Self::from_map(inner, timeline, map)
    }
    pub fn from_map(
        inner: Box<dyn AudioProcessor>,
        timeline: Timeline,
        map: Arc<TempoMap>,
    ) -> Result<Self, Error> {
        if TempoMap::compile(&timeline)? != *map {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            inner,
            timeline,
            map,
            clock: None,
            spec: None,
            scratch: vec![],
            source_end: None,
        })
    }
}
impl AudioProcessor for TimelineProcessor {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.spec = None;
        spec.validate()?;
        self.clock = Some(Clock::from_map(
            &self.timeline,
            spec.sample_rate,
            self.map.clone(),
        )?);
        self.inner.prepare(spec)?;
        self.scratch = vec![
            Event {
                frame: 0,
                message: MidiMessage::AllNotesOff
            };
            1024
        ];
        self.spec = Some(spec);
        self.source_end = None;
        Ok(())
    }
    fn reset(&mut self) {
        self.inner.reset();
        if let Some(c) = &mut self.clock {
            let _ = c.seek(0);
        }
        self.source_end = None;
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != spec.channels
            || b.frames() > spec.max_block_frames
            || ctx.events.len()
                > if self.timeline.panic_on_loop {
                    1023
                } else {
                    1024
                }
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        let clock = self.clock.as_mut().expect("prepared");
        if self.source_end != Some(ctx.transport.sample_position)
            && clock.seek(ctx.transport.sample_position).is_err()
        {
            b.clear();
            return;
        }
        clock.set_playing(ctx.transport.playing);
        let frames = b.frames();
        let mut event = 0;
        for (frame, samples) in b.samples_mut().chunks_exact_mut(spec.channels).enumerate() {
            let Ok(time) = clock.frame() else {
                samples.fill(0.0);
                continue;
            };
            let mut count = 0;
            if time.looped && self.timeline.panic_on_loop {
                self.scratch[count] = Event {
                    frame: 0,
                    message: MidiMessage::AllNotesOff,
                };
                count += 1;
            }
            while event < ctx.events.len() && ctx.events[event].frame == frame {
                self.scratch[count] = Event {
                    frame: 0,
                    message: ctx.events[event].message,
                };
                count += 1;
                event += 1;
            }
            self.inner.process(
                &mut AudioBuffer::new(samples, spec.channels).expect("prepared layout"),
                &ProcessContext {
                    events: &self.scratch[..count],
                    transport: time.core_transport(),
                },
            );
            if clock.advance(1).is_err() {
                samples.fill(0.0);
            }
        }
        self.source_end = Some(ctx.transport.sample_position.saturating_add(frames as u64));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn step_and_ramp_integrals_round_trip() {
        let d = Timeline {
            tempo: vec![
                TempoPoint {
                    beat: 0.,
                    bpm: 60.,
                    transition: Transition::Linear,
                },
                TempoPoint {
                    beat: 4.,
                    bpm: 120.,
                    transition: Transition::Step,
                },
            ],
            ..Default::default()
        };
        let m = TempoMap::compile(&d).unwrap();
        assert!((m.seconds_at(4.).unwrap() - 4. * 2.0f64.ln()).abs() < 1e-12);
        for b in [0., 0.5, 3.9, 4., 7., 100.] {
            let time = m.seconds_at(b).unwrap();
            assert!((m.beat_at_seconds(time).unwrap() - b).abs() < 1e-9);
        }
    }
    #[test]
    fn meter_changes_require_bar_boundaries() {
        let mut d = Timeline::default();
        d.meters.push(MeterPoint {
            beat: 4.,
            numerator: 7,
            denominator: 8,
        });
        let m = TempoMap::compile(&d).unwrap();
        let p = m.position(7.5).unwrap();
        assert_eq!((p.bar, p.numerator, p.denominator), (2, 7, 8));
        d.meters[1].beat = 3.;
        assert!(TempoMap::compile(&d).is_err());
    }
    #[test]
    fn loop_and_punch_use_sample_boundaries_without_allocating() {
        let d = Timeline {
            loop_range: Some([1., 2.]),
            punch_range: Some([1.5, 2.]),
            ..Default::default()
        };
        let mut c = Clock::new(&d, 8.).unwrap();
        c.set_playing(true);
        c.seek(6).unwrap();
        assert!(c.frame().unwrap().punching);
        let a = allocation_counter::measure(|| c.advance(2).unwrap());
        assert_eq!(a.count_total, 0);
        let f = c.frame().unwrap();
        assert!(f.looped);
        assert_eq!(f.sample, 4);
        assert!(!f.punching);
    }
}

#[cfg(test)]
mod processor_tests {
    use super::*;
    struct Mark;
    impl AudioProcessor for Mark {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn reset(&mut self) {}
        fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
            b.samples_mut().fill(c.transport.sample_position as f32);
        }
    }
    fn run(parts: &[usize]) -> Vec<f32> {
        let timeline = Timeline {
            loop_range: Some([1., 2.]),
            ..Default::default()
        };
        let mut p = TimelineProcessor::new(Box::new(Mark), timeline).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 8.,
            max_block_frames: 16,
            channels: 1,
        })
        .unwrap();
        let mut output = vec![];
        let mut at = 0;
        for n in parts {
            let mut data = vec![0.; *n];
            let count = allocation_counter::measure(|| {
                p.process(
                    &mut AudioBuffer::new(&mut data, 1).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport {
                            sample_position: at,
                            tempo: 120.,
                            playing: true,
                        },
                    },
                )
            });
            assert_eq!(count.count_total, 0);
            output.extend(data);
            at += *n as u64;
        }
        output
    }
    #[test]
    fn musical_loop_is_block_partition_invariant() {
        let a = run(&[16]);
        assert_eq!(a, run(&[3, 5, 8]));
        assert_eq!(
            a,
            &[0., 1., 2., 3., 4., 5., 6., 7., 4., 5., 6., 7., 4., 5., 6., 7.]
        );
    }
}
