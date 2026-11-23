//! String-free prepared modulation rendering with owned buffers and anchored control quanta.
use super::LfoShape;
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceKind {
    Constant {
        value: f32,
    },
    External,
    Lfo {
        shape: LfoShape,
        hz: f64,
        beats: Option<f64>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub kind: SourceKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub id: String,
    pub base: f32,
    pub smoothing_ms: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    Direct,
    BipolarToUnipolar,
    SignedSquare,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub source: String,
    pub destination: String,
    pub depth: f32,
    pub transform: Transform,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rate {
    Audio,
    Control { quantum: usize },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub sources: Vec<Source>,
    pub destinations: Vec<Destination>,
    pub routes: Vec<Route>,
    pub rate: Rate,
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct Stats {
    pub frames: u64,
    pub source_evaluations: u64,
    pub route_evaluations: u64,
}
pub struct Input<'a> {
    pub source: usize,
    pub values: &'a [f32],
}
struct State {
    phase: f64,
    random: u32,
    held: f32,
}
struct Target {
    base: f32,
    value: f64,
    target: f32,
    coefficient: f64,
    routes: Vec<(usize, f32, Transform)>,
}
pub struct CompiledMatrix {
    config: Config,
    kinds: Vec<SourceKind>,
    states: Vec<State>,
    targets: Vec<Target>,
    values: [f32; 64],
    pcm: Vec<f32>,
    maximum: usize,
    frames: usize,
    clock: u64,
    rate: f64,
    stats: Stats,
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}
impl CompiledMatrix {
    pub fn new(config: Config, sample_rate: f64, maximum_frames: usize) -> Result<Self, Error> {
        if config.schema != 1
            || !sample_rate.is_finite()
            || !(8000.0..=1536000.).contains(&sample_rate)
            || !(1..=8192).contains(&maximum_frames)
            || config.sources.len() > 64
            || config.sources.is_empty()
            || config.destinations.is_empty()
            || config.destinations.len() > 256
            || config.routes.len() > 2048
            || config.destinations.len() * maximum_frames > 1024 * 1024
        {
            return Err(Error::InvalidSpec);
        }
        match config.rate {
            Rate::Control { quantum } if !(1..=1024).contains(&quantum) => {
                return Err(Error::InvalidParameter)
            }
            _ => {}
        }
        let mut names = BTreeSet::new();
        for s in &config.sources {
            if !id(&s.id) || !names.insert(&s.id) {
                return Err(Error::InvalidParameter);
            }
            match s.kind {
                SourceKind::Constant { value }
                    if !value.is_finite() || !(-1.0..=1.).contains(&value) =>
                {
                    return Err(Error::InvalidParameter)
                }
                SourceKind::Lfo { hz, beats, .. }
                    if !hz.is_finite()
                        || !(0.0..=1000.).contains(&hz)
                        || beats
                            .is_some_and(|b| !b.is_finite() || !(0.015625..=64.).contains(&b)) =>
                {
                    return Err(Error::InvalidParameter)
                }
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        let mut targets = Vec::new();
        for d in &config.destinations {
            if !id(&d.id)
                || !names.insert(&d.id)
                || !d.base.is_finite()
                || !(0.0..=1.).contains(&d.base)
                || !d.smoothing_ms.is_finite()
                || !(0.0..=10000.).contains(&d.smoothing_ms)
            {
                return Err(Error::InvalidParameter);
            }
            targets.push(Target {
                base: d.base,
                value: d.base as f64,
                target: d.base,
                coefficient: if d.smoothing_ms == 0. {
                    1.
                } else {
                    -(-1. / (sample_rate * d.smoothing_ms * 0.001)).exp_m1()
                },
                routes: vec![],
            });
        }
        for r in &config.routes {
            if !r.depth.is_finite() || !(-2.0..=2.).contains(&r.depth) {
                return Err(Error::InvalidParameter);
            }
            let source = config
                .sources
                .iter()
                .position(|s| s.id == r.source)
                .ok_or(Error::InvalidRouting)?;
            let target = config
                .destinations
                .iter()
                .position(|d| d.id == r.destination)
                .ok_or(Error::InvalidRouting)?;
            targets[target].routes.push((source, r.depth, r.transform));
        }
        let kinds = config.sources.iter().map(|s| s.kind).collect();
        let states = (0..config.sources.len())
            .map(|i| State {
                phase: 0.,
                random: 0x69726f6e_u32.wrapping_add(i as u32),
                held: 0.,
            })
            .collect();
        let pcm = vec![0.; config.destinations.len() * maximum_frames];
        Ok(Self {
            config,
            kinds,
            states,
            targets,
            values: [0.; 64],
            pcm,
            maximum: maximum_frames,
            frames: 0,
            clock: 0,
            rate: sample_rate,
            stats: Stats::default(),
        })
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn source_index(&self, id: &str) -> Result<usize, Error> {
        self.config
            .sources
            .iter()
            .position(|s| s.id == id)
            .ok_or(Error::InvalidParameter)
    }
    pub fn destination_index(&self, id: &str) -> Result<usize, Error> {
        self.config
            .destinations
            .iter()
            .position(|s| s.id == id)
            .ok_or(Error::InvalidParameter)
    }
    pub fn set_base(&mut self, index: usize, value: f32) -> Result<(), Error> {
        if !value.is_finite() || !(0.0..=1.).contains(&value) {
            return Err(Error::InvalidParameter);
        }
        let t = self.targets.get_mut(index).ok_or(Error::InvalidParameter)?;
        t.base = value;
        self.config.destinations[index].base = value;
        Ok(())
    }
    pub fn set_constant(&mut self, index: usize, value: f32) -> Result<(), Error> {
        if !value.is_finite()
            || !(-1.0..=1.).contains(&value)
            || !self
                .kinds
                .get(index)
                .is_some_and(|k| matches!(k, SourceKind::Constant { .. }))
        {
            return Err(Error::InvalidParameter);
        }
        self.kinds[index] = SourceKind::Constant { value };
        self.config.sources[index].kind = self.kinds[index];
        Ok(())
    }
    pub fn set_lfo_rate(&mut self, index: usize, hz: f64, beats: Option<f64>) -> Result<(), Error> {
        if !hz.is_finite()
            || !(0.0..=1000.).contains(&hz)
            || beats.is_some_and(|b| !b.is_finite() || !(0.015625..=64.).contains(&b))
        {
            return Err(Error::InvalidParameter);
        }
        let SourceKind::Lfo { shape, .. } =
            *self.kinds.get(index).ok_or(Error::InvalidParameter)?
        else {
            return Err(Error::InvalidParameter);
        };
        self.kinds[index] = SourceKind::Lfo { shape, hz, beats };
        self.config.sources[index].kind = self.kinds[index];
        Ok(())
    }
    pub fn buffer(&self, index: usize) -> Result<&[f32], Error> {
        if index >= self.targets.len() {
            return Err(Error::InvalidParameter);
        }
        Ok(&self.pcm[index * self.maximum..index * self.maximum + self.frames])
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    pub fn reset(&mut self) {
        self.clock = 0;
        self.frames = 0;
        self.stats = Stats::default();
        self.values.fill(0.);
        self.pcm.fill(0.);
        for (i, s) in self.states.iter_mut().enumerate() {
            s.phase = 0.;
            s.held = 0.;
            s.random = 0x69726f6e_u32.wrapping_add(i as u32);
        }
        for t in &mut self.targets {
            t.value = t.base as f64;
            t.target = t.base;
        }
    }
    pub fn render(
        &mut self,
        frames: usize,
        tempo: f64,
        playing: bool,
        inputs: &[Input<'_>],
    ) -> Result<(), Error> {
        if frames > self.maximum
            || !tempo.is_finite()
            || !(1.0..=1000.).contains(&tempo)
            || self.clock.checked_add(frames as u64).is_none()
        {
            return Err(Error::InvalidBuffer);
        }
        let mut bindings: [Option<&[f32]>; 64] = [None; 64];
        for input in inputs {
            if input.source >= self.kinds.len()
                || !matches!(self.kinds[input.source], SourceKind::External)
                || bindings[input.source].is_some()
                || !(input.values.len() == 1 || input.values.len() == frames)
                || input
                    .values
                    .iter()
                    .any(|v| !v.is_finite() || !(-1.0..=1.).contains(v))
            {
                return Err(Error::InvalidParameter);
            }
            bindings[input.source] = Some(input.values);
        }
        for (i, kind) in self.kinds.iter().enumerate() {
            if matches!(kind, SourceKind::External) && bindings[i].is_none() {
                return Err(Error::InvalidParameter);
            }
        }
        let quantum = match self.config.rate {
            Rate::Audio => 1,
            Rate::Control { quantum } => quantum,
        } as u64;
        let mut increments = [0.; 64];
        for (i, kind) in self.kinds.iter().enumerate() {
            if let SourceKind::Lfo { hz, beats, .. } = kind {
                increments[i] = beats.map_or(*hz, |b| tempo / (60. * b)) / self.rate;
            }
        }
        for frame in 0..frames {
            if self.clock.is_multiple_of(quantum) {
                for (i, kind) in self.kinds.iter().enumerate() {
                    let s = &self.states[i];
                    self.values[i] = match *kind {
                        SourceKind::Constant { value } => value,
                        SourceKind::External => {
                            let v = bindings[i].expect("validated binding");
                            v[if v.len() == 1 { 0 } else { frame }]
                        }
                        SourceKind::Lfo { shape, .. } => match shape {
                            LfoShape::Sine => (std::f64::consts::TAU * s.phase).sin() as f32,
                            LfoShape::Triangle => (1. - 4. * (s.phase - 0.5).abs()) as f32,
                            LfoShape::Saw => (2. * s.phase - 1.) as f32,
                            LfoShape::Square => {
                                if s.phase < 0.5 {
                                    1.
                                } else {
                                    -1.
                                }
                            }
                            LfoShape::SampleHold => s.held,
                        },
                    };
                    self.stats.source_evaluations = self.stats.source_evaluations.saturating_add(1);
                }
                for t in &mut self.targets {
                    let mut value = t.base;
                    for &(source, depth, transform) in &t.routes {
                        let v = self.values[source];
                        let v = match transform {
                            Transform::Direct => v,
                            Transform::BipolarToUnipolar => 0.5 * (v + 1.),
                            Transform::SignedSquare => v * v.abs(),
                        };
                        value += depth * v;
                        self.stats.route_evaluations =
                            self.stats.route_evaluations.saturating_add(1);
                    }
                    t.target = value.clamp(0., 1.);
                }
            }
            for (i, t) in self.targets.iter_mut().enumerate() {
                t.value += t.coefficient * (t.target as f64 - t.value);
                self.pcm[i * self.maximum + frame] = t.value as f32;
            }
            if playing {
                for (i, s) in self.states.iter_mut().enumerate() {
                    s.phase += increments[i];
                    if s.phase >= 1. {
                        s.phase = s.phase.rem_euclid(1.);
                        s.random ^= s.random << 13;
                        s.random ^= s.random >> 17;
                        s.random ^= s.random << 5;
                        s.held = s.random as f32 / u32::MAX as f32 * 2. - 1.;
                    }
                }
            }
            self.clock += 1;
        }
        self.frames = frames;
        self.stats.frames = self.stats.frames.saturating_add(frames as u64);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            schema: 1,
            sources: vec![Source {
                id: "lfo".into(),
                kind: SourceKind::Lfo {
                    shape: LfoShape::Sine,
                    hz: 4.,
                    beats: None,
                },
            }],
            destinations: vec![Destination {
                id: "cutoff".into(),
                base: 0.5,
                smoothing_ms: 5.,
            }],
            routes: vec![Route {
                source: "lfo".into(),
                destination: "cutoff".into(),
                depth: 0.5,
                transform: Transform::Direct,
            }],
            rate: Rate::Control { quantum: 64 },
        }
    }
    #[test]
    fn partition_equality_allocations_and_evaluation_budget() {
        let render = |block: usize| {
            let mut m = CompiledMatrix::new(config(), 48000., 128).unwrap();
            let mut out = vec![];
            for _ in 0..1024 / block {
                assert_eq!(
                    allocation_counter::measure(|| m.render(block, 120., true, &[]).unwrap())
                        .count_total,
                    0
                );
                out.extend_from_slice(m.buffer(0).unwrap());
            }
            assert_eq!(m.stats().source_evaluations, 16);
            assert_eq!(m.stats().route_evaluations, 16);
            out
        };
        assert_eq!(render(128), render(1));
    }
    #[test]
    fn invalid_inputs_do_not_advance_or_change_base() {
        let mut c = config();
        c.sources[0].kind = SourceKind::External;
        let mut m = CompiledMatrix::new(c, 48000., 128).unwrap();
        assert!(m.render(128, 120., true, &[]).is_err());
        assert_eq!(m.stats().frames, 0);
        assert!(m.set_base(0, f32::NAN).is_err());
        assert_eq!(m.config().destinations[0].base, 0.5);
        m.render(
            128,
            120.,
            true,
            &[Input {
                source: 0,
                values: &[1.],
            }],
        )
        .unwrap();
        assert!(m.buffer(0).unwrap().iter().all(|v| *v >= 0.5 && *v <= 1.));
        m.reset();
        assert_eq!(m.stats().frames, 0);
    }
}
#[cfg(test)]
mod config_tests {
    use super::*;
    fn base() -> Config {
        Config {
            schema: 1,
            sources: vec![Source {
                id: "lfo".into(),
                kind: SourceKind::Lfo {
                    shape: LfoShape::Sine,
                    hz: 4.,
                    beats: None,
                },
            }],
            destinations: vec![Destination {
                id: "level".into(),
                base: 0.5,
                smoothing_ms: 0.,
            }],
            routes: vec![],
            rate: Rate::Audio,
        }
    }
    #[test]
    fn source_updates_preserve_phase_and_validate_topology() {
        let mut m = CompiledMatrix::new(base(), 48000., 128).unwrap();
        m.render(128, 120., true, &[]).unwrap();
        let phase = m.states[0].phase;
        m.set_lfo_rate(0, 8., Some(0.5)).unwrap();
        assert_eq!(phase, m.states[0].phase);
        assert!(m.set_constant(0, 0.1).is_err());
        assert!(m.set_lfo_rate(0, f64::NAN, None).is_err());
        let mut c = base();
        c.sources.push(c.sources[0].clone());
        assert!(CompiledMatrix::new(c, 48000., 128).is_err());
        let mut c = base();
        c.routes.push(Route {
            source: "missing".into(),
            destination: "level".into(),
            depth: 1.,
            transform: Transform::Direct,
        });
        assert!(CompiledMatrix::new(c, 48000., 128).is_err());
    }
    #[test]
    fn prepared_memory_budget_rejects_oversized_config() {
        let mut c = base();
        c.destinations = (0..256)
            .map(|i| Destination {
                id: format!("p{i}"),
                base: 0.,
                smoothing_ms: 0.,
            })
            .collect();
        assert!(CompiledMatrix::new(c, 48000., 8192).is_err());
    }
}
#[cfg(test)]
mod smoothing_tests {
    use super::*;
    #[test]
    fn slow_high_rate_smoothing_does_not_stall_at_f32_resolution() {
        let config = Config {
            schema: 1,
            sources: vec![Source {
                id: "constant".into(),
                kind: SourceKind::Constant { value: 1. },
            }],
            destinations: vec![Destination {
                id: "target".into(),
                base: 0.5,
                smoothing_ms: 10000.,
            }],
            routes: vec![Route {
                source: "constant".into(),
                destination: "target".into(),
                depth: 0.01,
                transform: Transform::Direct,
            }],
            rate: Rate::Audio,
        };
        let mut m = CompiledMatrix::new(config, 1536000., 8192).unwrap();
        m.render(8192, 120., true, &[]).unwrap();
        assert!(m.buffer(0).unwrap()[8191] > 0.500004);
    }
}
