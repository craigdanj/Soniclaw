//! Bounded global modulation. Base parameter values remain separate from offsets.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleHold,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeStage {
    pub seconds: f32,
    pub level: f32,
    pub curve: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceConfig {
    Lfo {
        shape: LfoShape,
        rate_hz: f32,
        beats: Option<f32>,
    },
    Envelope {
        stages: Vec<EnvelopeStage>,
        sustain_stage: usize,
    },
    Macro {
        index: usize,
    },
    MidiCc {
        channel: u8,
        controller: u8,
    },
    Pressure {
        channel: u8,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulationRoute {
    pub source: usize,
    pub destination: String,
    pub amount: f32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModulationConfig {
    pub sources: Vec<SourceConfig>,
    pub routes: Vec<ModulationRoute>,
    pub macros: Vec<f32>,
}
impl ModulationConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.sources.len() > 16
            || self.routes.len() > 64
            || self.macros.len() > 8
            || self
                .macros
                .iter()
                .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err(Error::InvalidParameter);
        }
        for source in &self.sources {
            match source {
                SourceConfig::Lfo { rate_hz, beats, .. } => {
                    if !rate_hz.is_finite()
                        || !(0.01..=40.0).contains(rate_hz)
                        || beats.is_some_and(|b| !b.is_finite() || !(0.0625..=16.0).contains(&b))
                    {
                        return Err(Error::InvalidParameter);
                    }
                }
                SourceConfig::Envelope {
                    stages,
                    sustain_stage,
                } => {
                    if stages.is_empty()
                        || stages.len() > 16
                        || *sustain_stage >= stages.len() - 1
                        || stages.iter().any(|s| {
                            !s.seconds.is_finite()
                                || !(0.001..=30.0).contains(&s.seconds)
                                || !s.level.is_finite()
                                || !(0.0..=1.0).contains(&s.level)
                                || !s.curve.is_finite()
                                || !(-4.0..=4.0).contains(&s.curve)
                        })
                    {
                        return Err(Error::InvalidParameter);
                    }
                }
                SourceConfig::Macro { index } => {
                    if *index >= self.macros.len() {
                        return Err(Error::InvalidParameter);
                    }
                }
                SourceConfig::MidiCc {
                    channel,
                    controller,
                } => {
                    if *channel > 15 || *controller > 127 {
                        return Err(Error::InvalidParameter);
                    }
                }
                SourceConfig::Pressure { channel } => {
                    if *channel > 15 {
                        return Err(Error::InvalidParameter);
                    }
                }
            }
        }
        for r in &self.routes {
            if r.source >= self.sources.len()
                || r.destination.is_empty()
                || !r.amount.is_finite()
                || !(-1.0..=1.0).contains(&r.amount)
            {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct State {
    phase: f64,
    value: f32,
    start: f32,
    stage: usize,
    elapsed: usize,
    held: bool,
    active: bool,
    random: u32,
}
impl Default for State {
    fn default() -> Self {
        Self {
            phase: 0.0,
            value: 0.0,
            start: 0.0,
            stage: 0,
            elapsed: 0,
            held: false,
            active: false,
            random: 0x6c6162,
        }
    }
}
struct Route {
    source: usize,
    target: usize,
    amount: f32,
}
pub struct ModulationMatrix {
    config: ModulationConfig,
    states: Vec<State>,
    targets: Vec<Arc<Parameter>>,
    routes: Vec<Route>,
    macros: Vec<Arc<Parameter>>,
    sr: f64,
    values: [f32; 16],
}
impl ModulationMatrix {
    pub fn new(
        config: ModulationConfig,
        parameters: &[Arc<Parameter>],
        reuse: &[Arc<Parameter>],
    ) -> Result<Self, Error> {
        config.validate()?;
        let mut targets: Vec<Arc<Parameter>> = vec![];
        let mut routes = vec![];
        for r in &config.routes {
            let p = parameters
                .iter()
                .find(|p| p.info().id == r.destination)
                .ok_or(Error::InvalidParameter)?;
            let target = if let Some(i) = targets.iter().position(|q| Arc::ptr_eq(q, p)) {
                i
            } else {
                if targets.len() == 64 {
                    return Err(Error::InvalidParameter);
                }
                targets.push(p.clone());
                targets.len() - 1
            };
            routes.push(Route {
                source: r.source,
                target,
                amount: r.amount,
            });
        }
        let mut macros = vec![];
        for (i, value) in config.macros.iter().enumerate() {
            let id = format!("mod.macro{}", i + 1);
            if parameters.iter().any(|p| p.info().id == id) {
                return Err(Error::InvalidParameter);
            }
            let p = if let Some(p) = reuse.iter().find(|p| p.info().id == id) {
                if p.info().min != 0.0 || p.info().max != 1.0 {
                    return Err(Error::InvalidParameter);
                }
                p.clone()
            } else {
                Arc::new(Parameter::new(ParameterInfo {
                    id: &id,
                    name: &format!("Macro {}", i + 1),
                    min: 0.0,
                    max: 1.0,
                    default: *value,
                    unit: "",
                })?)
            };
            macros.push(p);
        }
        let states = vec![State::default(); config.sources.len()];
        Ok(Self {
            config,
            states,
            targets,
            routes,
            macros,
            sr: 48000.0,
            values: [0.0; 16],
        })
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.macros
    }
    pub fn prepare(&mut self, sr: f64) -> Result<(), Error> {
        if !sr.is_finite() || !(8000.0..=192000.0).contains(&sr) {
            return Err(Error::InvalidSpec);
        }
        self.sr = sr;
        self.reset();
        Ok(())
    }
    pub fn source_values(&self) -> &[f32] {
        &self.values[..self.states.len()]
    }
    pub fn message(&mut self, m: MidiMessage) {
        for (source, state) in self.config.sources.iter().zip(&mut self.states) {
            match (source, m) {
                (
                    SourceConfig::Envelope { .. },
                    MidiMessage::NoteOn {
                        velocity: 1..=127, ..
                    },
                )
                | (
                    SourceConfig::Envelope { .. },
                    MidiMessage::NoteOnId {
                        velocity: 0.000001..=1.0,
                        ..
                    },
                ) => {
                    state.start = 0.0;
                    state.value = 0.0;
                    state.stage = 0;
                    state.elapsed = 0;
                    state.held = true;
                    state.active = true;
                }
                (
                    SourceConfig::Envelope {
                        stages,
                        sustain_stage,
                    },
                    MidiMessage::NoteOff { .. }
                    | MidiMessage::NoteOffId { .. }
                    | MidiMessage::NoteOn { velocity: 0, .. }
                    | MidiMessage::NoteOnId { velocity: 0.0, .. },
                ) => {
                    state.held = false;
                    if state.stage <= *sustain_stage {
                        state.stage = (*sustain_stage + 1).min(stages.len() - 1);
                        state.start = state.value;
                        state.elapsed = 0;
                    }
                }
                (
                    SourceConfig::MidiCc {
                        channel,
                        controller,
                    },
                    MidiMessage::ControlChange {
                        channel: c,
                        controller: k,
                        value,
                    },
                ) if *channel == c && *controller == k => state.value = value as f32 / 127.0,
                (
                    SourceConfig::Pressure { channel },
                    MidiMessage::ChannelPressure { channel: c, value },
                ) if *channel == c => state.value = value as f32 / 127.0,
                (
                    SourceConfig::Pressure { channel },
                    MidiMessage::NoteExpression {
                        target,
                        expression: NoteExpression::Pressure,
                        value,
                    },
                ) if target.channel < 0 || target.channel == *channel as i16 => {
                    state.value = (value as f32).clamp(0.0, 1.0)
                }
                (_, MidiMessage::AllNotesOff | MidiMessage::NoteChoke { .. }) => {
                    *state = State::default()
                }
                _ => {}
            }
        }
    }
    pub fn tick(&mut self, tempo: f64, playing: bool) {
        let tempo = if tempo.is_finite() {
            tempo.clamp(20.0, 400.0)
        } else {
            120.0
        };
        for (i, (source, s)) in self.config.sources.iter().zip(&mut self.states).enumerate() {
            self.values[i] = match source {
                SourceConfig::Lfo {
                    shape,
                    rate_hz,
                    beats,
                } => {
                    let phase = s.phase as f32;
                    let v = match shape {
                        LfoShape::Sine => (std::f32::consts::TAU * phase).sin(),
                        LfoShape::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                        LfoShape::Saw => 2.0 * phase - 1.0,
                        LfoShape::Square => {
                            if phase < 0.5 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        LfoShape::SampleHold => s.value,
                    };
                    if playing {
                        let hz = beats.map_or(*rate_hz as f64, |b| tempo / (60.0 * b as f64));
                        s.phase += hz / self.sr;
                        if s.phase >= 1.0 {
                            s.phase = s.phase.rem_euclid(1.0);
                            s.random ^= s.random << 13;
                            s.random ^= s.random >> 17;
                            s.random ^= s.random << 5;
                            s.value = s.random as f32 / u32::MAX as f32 * 2.0 - 1.0;
                        }
                    }
                    v
                }
                SourceConfig::Envelope {
                    stages,
                    sustain_stage,
                } => {
                    if s.active && playing {
                        if s.stage == *sustain_stage
                            && s.held
                            && s.elapsed
                                >= ((stages[s.stage].seconds as f64 * self.sr).round() as usize)
                                    .max(1)
                        {
                        } else {
                            let stage = &stages[s.stage];
                            let frames = (stage.seconds as f64 * self.sr).round().max(1.0) as usize;
                            s.elapsed = (s.elapsed + 1).min(frames);
                            let t =
                                (s.elapsed as f32 / frames as f32).powf(2.0_f32.powf(stage.curve));
                            s.value = s.start + (stage.level - s.start) * t;
                            if s.elapsed == frames && !(s.stage == *sustain_stage && s.held) {
                                if s.stage + 1 < stages.len() {
                                    s.stage += 1;
                                    s.elapsed = 0;
                                    s.start = s.value;
                                } else {
                                    s.active = false;
                                }
                            }
                        }
                    }
                    s.value
                }
                SourceConfig::Macro { index } => self.macros[*index].get(),
                SourceConfig::MidiCc { .. } | SourceConfig::Pressure { .. } => s.value,
            };
        }
        let mut offsets = [0.0; 64];
        for r in &self.routes {
            offsets[r.target] += self.values[r.source] * r.amount;
        }
        for (p, v) in self.targets.iter().zip(offsets) {
            let _ = p.set_modulation(v.clamp(-1.0, 1.0));
        }
    }
    pub fn reset(&mut self) {
        self.states.fill(State::default());
        self.values.fill(0.0);
        for p in &self.targets {
            let _ = p.set_modulation(0.0);
        }
    }
}
pub struct ModulatedProcessor {
    inner: Box<dyn AudioProcessor>,
    matrix: ModulationMatrix,
    scratch: Vec<Event>,
}
impl ModulatedProcessor {
    pub fn new(inner: Box<dyn AudioProcessor>, matrix: ModulationMatrix) -> Self {
        Self {
            inner,
            matrix,
            scratch: vec![],
        }
    }
}
impl AudioProcessor for ModulatedProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.inner.prepare(s)?;
        self.matrix.prepare(s.sample_rate)?;
        self.scratch = vec![
            Event {
                frame: 0,
                message: MidiMessage::AllNotesOff
            };
            1024
        ];
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if ctx.events.len() > self.scratch.len() {
            b.clear();
            return;
        }
        let ch = b.channels();
        let mut event = 0;
        for (i, frame) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            let mut n = 0;
            while event < ctx.events.len() && ctx.events[event].frame == i {
                let m = ctx.events[event].message;
                self.matrix.message(m);
                self.scratch[n] = Event {
                    frame: 0,
                    message: m,
                };
                n += 1;
                event += 1;
            }
            self.matrix.tick(ctx.transport.tempo, ctx.transport.playing);
            self.inner.process(
                &mut AudioBuffer::new(frame, ch).expect("complete frame"),
                &ProcessContext {
                    events: &self.scratch[..n],
                    transport: Transport {
                        sample_position: ctx.transport.sample_position + i as u64,
                        ..ctx.transport
                    },
                },
            );
        }
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.matrix.reset();
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base_is_preserved_and_modulation_bounded() {
        let p = Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                unit: "",
            })
            .unwrap(),
        );
        let config = ModulationConfig {
            sources: vec![SourceConfig::Macro { index: 0 }],
            routes: vec![ModulationRoute {
                source: 0,
                destination: "gain".into(),
                amount: 0.4,
            }],
            macros: vec![0.5],
        };
        let mut m = ModulationMatrix::new(config, std::slice::from_ref(&p), &[]).unwrap();
        m.prepare(48000.0).unwrap();
        m.tick(120.0, true);
        assert_eq!(p.get(), 0.5);
        assert!((p.modulated() - 0.7).abs() < 1e-6);
        m.parameters()[0].set(1.0).unwrap();
        m.tick(120.0, true);
        assert!((p.modulated() - 0.9).abs() < 1e-6);
        m.reset();
        assert_eq!(p.modulated(), 0.5);
    }
    #[test]
    fn envelope_releases_from_current_level() {
        let config = ModulationConfig {
            sources: vec![SourceConfig::Envelope {
                stages: vec![
                    EnvelopeStage {
                        seconds: 0.01,
                        level: 1.0,
                        curve: 0.0,
                    },
                    EnvelopeStage {
                        seconds: 0.01,
                        level: 0.5,
                        curve: 0.0,
                    },
                    EnvelopeStage {
                        seconds: 0.01,
                        level: 0.0,
                        curve: 0.0,
                    },
                ],
                sustain_stage: 1,
            }],
            ..Default::default()
        };
        let mut m = ModulationMatrix::new(config, &[], &[]).unwrap();
        m.prepare(8000.0).unwrap();
        m.message(MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 127,
        });
        for _ in 0..40 {
            m.tick(120.0, true);
        }
        assert!((m.source_values()[0] - 0.5).abs() < 1e-6);
        m.message(MidiMessage::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        });
        for _ in 0..80 {
            m.tick(120.0, true);
        }
        assert_eq!(m.source_values()[0], 0.0);
    }
}

pub mod compiled;
