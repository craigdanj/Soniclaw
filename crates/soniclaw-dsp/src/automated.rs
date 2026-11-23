//! Shared atomic control API for live effects. Structural edits/IR replacement
//! still require stopping; scalar control changes are smoothed per audio frame.
use crate::advanced::{
    creative::*, crossover::CrossoverBand, dynamics::*, filters::*, reverb::FdnReverb,
    spectral::SpectralGate, LiveDsp,
};
use crate::*;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Multiband,
    DynamicEq,
    SpectralFreeze,
    Robotisation,
    PitchShift,
    FrequencyShift,
    Gain,
    LowPass,
    Eq,
    Compressor,
    Gate,
    Chorus,
    Reverb,
    Limiter,
    Drive,
    Delay,
    StereoWidth,
    StateVariable,
    Ladder,
    CrossoverLow,
    CrossoverMid,
    CrossoverHigh,
    RmsCompressor,
    LookaheadLimiter,
    TransientShaper,
    SpectralGate,
    FdnReverb,
    Phaser,
    Flanger,
    Tremolo,
    AutoPan,
    MultiTapDelay,
}
type ControlSpec = (&'static str, &'static str, f32, f32, f32, &'static str);
impl EffectKind {
    fn specs(self) -> &'static [ControlSpec] {
        match self {
            Self::Multiband => &[
                ("low_hz", "Low crossover", 40.0, 1000.0, 300.0, "Hz"),
                ("high_hz", "High crossover", 1200.0, 16000.0, 3000.0, "Hz"),
                ("low_threshold", "Low Threshold", -60.0, 0.0, -18.0, "dB"),
                ("low_ratio", "Low Ratio", 1.0, 20.0, 1.0, ""),
                ("low_gain", "Low Gain", -24.0, 24.0, 0.0, "dB"),
                ("low_drive", "Low Drive", 0.0, 12.0, 0.0, ""),
                ("low_mute", "Low Mute", 0.0, 1.0, 0.0, ""),
                ("low_solo", "Low Solo", 0.0, 1.0, 0.0, ""),
                ("mid_threshold", "Mid Threshold", -60.0, 0.0, -18.0, "dB"),
                ("mid_ratio", "Mid Ratio", 1.0, 20.0, 1.0, ""),
                ("mid_gain", "Mid Gain", -24.0, 24.0, 0.0, "dB"),
                ("mid_drive", "Mid Drive", 0.0, 12.0, 0.0, ""),
                ("mid_mute", "Mid Mute", 0.0, 1.0, 0.0, ""),
                ("mid_solo", "Mid Solo", 0.0, 1.0, 0.0, ""),
                ("high_threshold", "High Threshold", -60.0, 0.0, -18.0, "dB"),
                ("high_ratio", "High Ratio", 1.0, 20.0, 1.0, ""),
                ("high_gain", "High Gain", -24.0, 24.0, 0.0, "dB"),
                ("high_drive", "High Drive", 0.0, 12.0, 0.0, ""),
                ("high_mute", "High Mute", 0.0, 1.0, 0.0, ""),
                ("high_solo", "High Solo", 0.0, 1.0, 0.0, ""),
            ],
            Self::DynamicEq => &[
                ("frequency", "Frequency", 20.0, 20000.0, 1000.0, "Hz"),
                ("q", "Q", 0.1, 10.0, 1.0, ""),
                ("threshold", "Threshold", -60.0, 0.0, -24.0, "dB"),
                ("range", "Dynamic range", -24.0, 24.0, -12.0, "dB"),
                ("gain", "Static gain", -12.0, 12.0, 0.0, "dB"),
                ("attack_ms", "Attack", 1.0, 200.0, 10.0, "ms"),
                ("release_ms", "Release", 10.0, 2000.0, 100.0, "ms"),
            ],
            Self::SpectralFreeze => &[
                ("hold", "Freeze", 0.0, 1.0, 0.0, ""),
                ("mix", "Mix", 0.0, 1.0, 1.0, ""),
            ],
            Self::Robotisation => &[
                ("enable", "Robot", 0.0, 1.0, 1.0, ""),
                ("mix", "Mix", 0.0, 1.0, 1.0, ""),
            ],
            Self::PitchShift => &[
                ("semitones", "Pitch", -24.0, 24.0, 0.0, "st"),
                ("mix", "Mix", 0.0, 1.0, 1.0, ""),
            ],
            Self::FrequencyShift => &[
                ("shift_hz", "Shift", -2000.0, 2000.0, 0.0, "Hz"),
                ("mix", "Mix", 0.0, 1.0, 1.0, ""),
            ],
            Self::StereoWidth => &[("width", "Width", 0.0, 2.0, 1.0, "")],
            Self::StateVariable => &[
                ("frequency", "Cutoff", 20.0, 20000.0, 1000.0, "Hz"),
                ("q", "Q", 0.1, 20.0, 0.707, ""),
                ("drive", "Drive", 0.0, 12.0, 0.0, ""),
                ("mode", "Mode", 0.0, 3.0, 0.0, ""),
            ],
            Self::Ladder => &[
                ("frequency", "Cutoff", 20.0, 20000.0, 1000.0, "Hz"),
                ("resonance", "Resonance", 0.0, 0.98, 0.3, ""),
                ("drive", "Drive", 0.0, 12.0, 1.0, ""),
            ],
            Self::CrossoverLow | Self::CrossoverMid | Self::CrossoverHigh => &[
                ("low_hz", "Low crossover", 40.0, 1000.0, 300.0, "Hz"),
                ("high_hz", "High crossover", 1200.0, 16000.0, 3000.0, "Hz"),
            ],
            Self::RmsCompressor => &[
                ("threshold_db", "Threshold", -100.0, 0.0, -18.0, "dB"),
                ("ratio", "Ratio", 1.0, 100.0, 4.0, ""),
                ("attack_ms", "Attack", 0.0, 1000.0, 10.0, "ms"),
                ("release_ms", "Release", 1.0, 10000.0, 100.0, "ms"),
                ("makeup_db", "Makeup", -36.0, 36.0, 0.0, "dB"),
                ("window_ms", "RMS window", 1.0, 100.0, 10.0, "ms"),
            ],
            Self::LookaheadLimiter => &[
                ("ceiling_db", "Ceiling", -24.0, 0.0, -1.0, "dBTP"),
                ("release_ms", "Release", 1.0, 2000.0, 100.0, "ms"),
            ],
            Self::TransientShaper => &[
                ("attack_db", "Attack", -24.0, 24.0, 0.0, "dB"),
                ("sustain_db", "Sustain", -24.0, 24.0, 0.0, "dB"),
            ],
            Self::SpectralGate => &[
                ("threshold_db", "Threshold", -100.0, 0.0, -60.0, "dB"),
                ("mix", "Mix", 0.0, 1.0, 1.0, ""),
            ],
            Self::FdnReverb => &[
                ("rt60", "Decay", 0.1, 15.0, 2.0, "s"),
                ("mix", "Mix", 0.0, 1.0, 0.25, ""),
                ("damping_hz", "Damping", 500.0, 16000.0, 6000.0, "Hz"),
                ("predelay_ms", "Predelay", 0.0, 200.0, 15.0, "ms"),
                ("diffusion", "Diffusion", 0.0, 1.0, 0.7, ""),
                ("modulation", "Modulation", 0.0, 1.0, 0.3, ""),
            ],
            Self::Phaser => &[
                ("rate_hz", "Rate", 0.05, 5.0, 0.3, "Hz"),
                ("depth", "Depth", 0.0, 1.0, 0.7, ""),
                ("feedback", "Feedback", -0.8, 0.8, 0.2, ""),
                ("mix", "Mix", 0.0, 1.0, 0.5, ""),
            ],
            Self::Flanger => &[
                ("rate_hz", "Rate", 0.05, 5.0, 0.25, "Hz"),
                ("depth_ms", "Depth", 0.0, 5.0, 3.0, "ms"),
                ("feedback", "Feedback", -0.9, 0.9, 0.3, ""),
                ("mix", "Mix", 0.0, 1.0, 0.5, ""),
            ],
            Self::Tremolo | Self::AutoPan => &[
                ("rate_hz", "Rate", 0.05, 20.0, 4.0, "Hz"),
                ("depth", "Depth", 0.0, 1.0, 0.5, ""),
            ],
            Self::MultiTapDelay => &[
                ("beats", "Beat length", 0.0625, 2.0, 0.5, "beats"),
                ("feedback", "Feedback", 0.0, 0.9, 0.3, ""),
                ("mix", "Mix", 0.0, 1.0, 0.3, ""),
            ],
            Self::Gain => &[("gain", "Gain", 0.0, 4.0, 1.0, "")],
            Self::LowPass => &[
                ("frequency", "Cutoff", 20.0, 20000.0, 3000.0, "Hz"),
                ("q", "Resonance", 0.1, 10.0, 0.707, ""),
            ],
            Self::Eq => &[
                ("frequency", "Frequency", 20.0, 20000.0, 1000.0, "Hz"),
                ("q", "Q", 0.1, 10.0, 1.0, ""),
                ("gain_db", "Gain", -36.0, 36.0, 0.0, "dB"),
            ],
            Self::Compressor => &[
                ("threshold_db", "Threshold", -100.0, 0.0, -18.0, "dB"),
                ("ratio", "Ratio", 1.0, 100.0, 4.0, ""),
                ("knee_db", "Knee", 0.0, 36.0, 6.0, "dB"),
                ("attack_ms", "Attack", 0.0, 1000.0, 10.0, "ms"),
                ("release_ms", "Release", 1.0, 10000.0, 100.0, "ms"),
                ("makeup_db", "Makeup", -36.0, 36.0, 0.0, "dB"),
            ],
            Self::Gate => &[
                ("threshold_db", "Threshold", -120.0, 0.0, -40.0, "dB"),
                ("attack_ms", "Attack", 0.0, 1000.0, 1.0, "ms"),
                ("hold_ms", "Hold", 0.0, 10000.0, 10.0, "ms"),
                ("release_ms", "Release", 1.0, 10000.0, 100.0, "ms"),
            ],
            Self::Chorus => &[
                ("rate_hz", "Rate", 0.01, 10.0, 0.7, "Hz"),
                ("depth_ms", "Depth", 0.0, 12.0, 3.0, "ms"),
                ("mix", "Mix", 0.0, 1.0, 0.15, ""),
            ],
            Self::Reverb => &[
                ("rt60", "Decay", 0.1, 20.0, 0.8, "s"),
                ("mix", "Mix", 0.0, 1.0, 0.12, ""),
            ],
            Self::Limiter => &[
                ("ceiling_db", "Ceiling", -60.0, 0.0, -1.0, "dB"),
                ("release_ms", "Release", 1.0, 10000.0, 80.0, "ms"),
            ],
            Self::Drive => &[
                ("drive", "Drive", 0.0, 32.0, 2.0, ""),
                ("mix", "Mix", 0.0, 1.0, 0.5, ""),
            ],
            Self::Delay => &[
                ("time_ms", "Time", 1.0, 2000.0, 250.0, "ms"),
                ("feedback", "Feedback", 0.0, 0.95, 0.3, ""),
                ("mix", "Mix", 0.0, 1.0, 0.2, ""),
            ],
        }
    }
}
enum Processor {
    Advanced(Box<dyn LiveDsp>),
    Gain,
    Eq(ParametricEq),
    Compressor(Compressor),
    Gate(NoiseGate),
    Chorus(Chorus),
    Reverb(Reverb),
    Limiter(PeakLimiter),
    Drive(Box<OversampledDrive>),
    Delay,
}
pub struct AutomatedEffect {
    kind: EffectKind,
    parameters: Vec<Arc<Parameter>>,
    smooth: Vec<SmoothedValue>,
    processor: Processor,
    sr: f32,
    channels: usize,
    cached: [f32; 32],
    delay: Vec<f32>,
    cursor: usize,
    delay_frames: usize,
    prepared: bool,
}
impl AutomatedEffect {
    pub fn new(kind: EffectKind, prefix: &str) -> Result<Self, Error> {
        if prefix.is_empty() {
            return Err(Error::InvalidParameter);
        }
        let parameters = kind
            .specs()
            .iter()
            .map(|&(id, name, min, max, default, unit)| {
                Ok(Arc::new(Parameter::new(ParameterInfo {
                    id: &format!("{prefix}.{id}"),
                    name,
                    min,
                    max,
                    default,
                    unit,
                })?))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let processor = match kind {
            EffectKind::Multiband => {
                Processor::Advanced(Box::new(crate::rack::MultibandRack::default()))
            }
            EffectKind::DynamicEq => {
                Processor::Advanced(Box::new(crate::rack::DynamicEq::default()))
            }
            EffectKind::SpectralFreeze => Processor::Advanced(Box::new(
                crate::spectral_fx::SpectralEffect::new(crate::spectral_fx::SpectralMode::Freeze),
            )),
            EffectKind::Robotisation => Processor::Advanced(Box::new(
                crate::spectral_fx::SpectralEffect::new(crate::spectral_fx::SpectralMode::Robot),
            )),
            EffectKind::PitchShift => {
                Processor::Advanced(Box::new(crate::spectral_fx::SpectralEffect::new(
                    crate::spectral_fx::SpectralMode::PitchShift,
                )))
            }
            EffectKind::FrequencyShift => {
                Processor::Advanced(Box::new(crate::spectral_fx::FrequencyShifter::default()))
            }
            EffectKind::StereoWidth => {
                Processor::Advanced(Box::new(crate::advanced::analysis::StereoWidth::default()))
            }
            EffectKind::StateVariable => {
                Processor::Advanced(Box::new(StateVariableFilter::default()))
            }
            EffectKind::Ladder => Processor::Advanced(Box::new(LadderFilter::default())),
            EffectKind::CrossoverLow => Processor::Advanced(Box::new(CrossoverBand::new(0)?)),
            EffectKind::CrossoverMid => Processor::Advanced(Box::new(CrossoverBand::new(1)?)),
            EffectKind::CrossoverHigh => Processor::Advanced(Box::new(CrossoverBand::new(2)?)),
            EffectKind::RmsCompressor => Processor::Advanced(Box::new(RmsCompressor::default())),
            EffectKind::LookaheadLimiter => {
                Processor::Advanced(Box::new(LookaheadLimiter::default()))
            }
            EffectKind::TransientShaper => {
                Processor::Advanced(Box::new(TransientShaper::default()))
            }
            EffectKind::SpectralGate => Processor::Advanced(Box::new(SpectralGate::default())),
            EffectKind::FdnReverb => Processor::Advanced(Box::new(FdnReverb::default())),
            EffectKind::Phaser => Processor::Advanced(Box::new(Phaser::default())),
            EffectKind::Flanger => Processor::Advanced(Box::new(Flanger::default())),
            EffectKind::Tremolo => Processor::Advanced(Box::new(AmplitudeModulator::tremolo())),
            EffectKind::AutoPan => Processor::Advanced(Box::new(AmplitudeModulator::autopan())),
            EffectKind::MultiTapDelay => Processor::Advanced(Box::new(MultiTapDelay::default())),
            EffectKind::Gain => Processor::Gain,
            EffectKind::LowPass | EffectKind::Eq => {
                Processor::Eq(ParametricEq::new(vec![EqBand {
                    shape: if kind == EffectKind::LowPass {
                        EqShape::LowPass
                    } else {
                        EqShape::Peak
                    },
                    frequency: 1000.0,
                    q: 0.707,
                    gain_db: 0.0,
                }])?)
            }
            EffectKind::Compressor => Processor::Compressor(Compressor::new(Default::default())?),
            EffectKind::Gate => Processor::Gate(NoiseGate::new(-40.0, 1.0, 10.0, 100.0)?),
            EffectKind::Chorus => Processor::Chorus(Chorus::new(0.7, 3.0, 0.15)?),
            EffectKind::Reverb => Processor::Reverb(Reverb::new(0.8, 0.12)?),
            EffectKind::Limiter => Processor::Limiter(PeakLimiter::new(-1.0, 80.0)?),
            EffectKind::Drive => Processor::Drive(Box::new(OversampledDrive::new(2.0, 0.5)?)),
            EffectKind::Delay => Processor::Delay,
        };
        Ok(Self {
            kind,
            parameters,
            smooth: vec![],
            processor,
            sr: 48000.0,
            channels: 0,
            cached: [f32::NAN; 32],
            delay: vec![],
            cursor: 0,
            delay_frames: 0,
            prepared: false,
        })
    }
    /// Reuse host-visible scalar identities when rebuilding a stopped project.
    pub fn reuse_parameters(&mut self, existing: &[Arc<Parameter>]) -> Result<(), Error> {
        for p in &mut self.parameters {
            if let Some(old) = existing.iter().find(|q| q.info().id == p.info().id) {
                let (a, b) = (old.info(), p.info());
                if a.min != b.min || a.max != b.max {
                    return Err(Error::InvalidParameter);
                }
                *p = old.clone();
            } else if !existing.is_empty() {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    fn update(&mut self, v: [f32; 32]) {
        match &mut self.processor {
            Processor::Advanced(p) => p.update(&v),
            Processor::Eq(p) => p.update_band(
                0,
                EqBand {
                    shape: if self.kind == EffectKind::LowPass {
                        EqShape::LowPass
                    } else {
                        EqShape::Peak
                    },
                    frequency: v[0],
                    q: v[1],
                    gain_db: if self.kind == EffectKind::LowPass {
                        0.0
                    } else {
                        v[2]
                    },
                },
                self.sr,
            ),
            Processor::Compressor(p) => p.update_live(
                CompressorSettings {
                    threshold_db: v[0],
                    ratio: v[1],
                    knee_db: v[2],
                    attack_ms: v[3],
                    release_ms: v[4],
                    makeup_db: v[5],
                },
                self.sr,
            ),
            Processor::Gate(p) => p.update_live(v[0], v[1], v[2], v[3], self.sr),
            Processor::Chorus(p) => p.update_live(v[0], v[1], v[2]),
            Processor::Reverb(p) => p.update_live(v[0], v[1], self.sr),
            Processor::Limiter(p) => p.update_live(v[0], v[1], self.sr),
            Processor::Drive(p) => p.update_live(v[0], v[1]),
            _ => {}
        }
    }
}
impl AudioProcessor for AutomatedEffect {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.prepared = false;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        if s.sample_rate * s.channels as f64 > 8_000_000.0 {
            return Err(Error::InvalidSpec);
        }
        self.smooth = self
            .parameters
            .iter()
            .map(|p| SmoothedValue::new(p.modulated(), (self.sr * 0.01).ceil() as usize))
            .collect();
        match &mut self.processor {
            Processor::Advanced(p) => p.prepare(s)?,
            Processor::Eq(p) => p.prepare(s)?,
            Processor::Compressor(p) => p.prepare(s)?,
            Processor::Gate(p) => p.prepare(s)?,
            Processor::Chorus(p) => p.prepare(s)?,
            Processor::Reverb(p) => p.prepare(s)?,
            Processor::Limiter(p) => p.prepare(s)?,
            Processor::Drive(p) => p.prepare(s)?,
            Processor::Delay => {
                self.delay_frames = (self.sr * 2.0).ceil() as usize + 2;
                self.delay = vec![0.0; self.delay_frames * s.channels];
            }
            _ => {}
        }
        self.prepared = true;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if !self.prepared || b.channels() != self.channels {
            b.clear();
            return;
        }
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.set_target(p.modulated());
        }
        let count = self.smooth.len();
        let mut v = [0.0; 32];
        // Constant controls need one update and one inner call for the whole block.
        if !matches!(self.processor, Processor::Delay)
            && self.smooth.iter().all(|s| s.settled_value().is_some())
        {
            for (out, smooth) in v.iter_mut().zip(&self.smooth) {
                *out = smooth.settled_value().expect("settled control");
            }
            if self.cached[..count] != v[..count] {
                self.update(v);
                self.cached = v;
            }
            let processor: &mut dyn AudioProcessor = match &mut self.processor {
                Processor::Gain => {
                    for x in b.samples_mut() {
                        *x *= v[0];
                    }
                    return;
                }
                Processor::Advanced(p) => p.as_mut(),
                Processor::Eq(p) => p,
                Processor::Compressor(p) => p,
                Processor::Gate(p) => p,
                Processor::Chorus(p) => p,
                Processor::Reverb(p) => p,
                Processor::Limiter(p) => p,
                Processor::Drive(p) => p.as_mut(),
                Processor::Delay => unreachable!(),
            };
            processor.process(
                b,
                &ProcessContext {
                    events: &[],
                    transport: ctx.transport,
                },
            );
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            for (out, s) in v.iter_mut().zip(&mut self.smooth) {
                *out = s.next_value();
            }
            if self.cached[..count] != v[..count] {
                self.update(v);
                self.cached = v;
            }
            match &mut self.processor {
                Processor::Gain => {
                    for x in frame {
                        *x *= v[0];
                    }
                }
                Processor::Delay => {
                    let distance = v[0] * self.sr * 0.001;
                    let pos = (self.cursor as f32 - distance).rem_euclid(self.delay_frames as f32);
                    let a = pos as usize;
                    let frac = pos.fract();
                    for (c, x) in frame.iter_mut().enumerate() {
                        let wet = self.delay[a * self.channels + c] * (1.0 - frac)
                            + self.delay[((a + 1) % self.delay_frames) * self.channels + c] * frac;
                        self.delay[self.cursor * self.channels + c] = *x + wet * v[1];
                        *x = *x * (1.0 - v[2]) + wet * v[2];
                    }
                    self.cursor = (self.cursor + 1) % self.delay_frames;
                }
                other => {
                    let p: &mut dyn AudioProcessor = match other {
                        Processor::Advanced(p) => p.as_mut(),
                        Processor::Eq(p) => p,
                        Processor::Compressor(p) => p,
                        Processor::Gate(p) => p,
                        Processor::Chorus(p) => p,
                        Processor::Reverb(p) => p,
                        Processor::Limiter(p) => p,
                        Processor::Drive(p) => p.as_mut(),
                        _ => unreachable!(),
                    };
                    p.process(
                        &mut AudioBuffer::new(frame, self.channels).expect("complete frame"),
                        &ProcessContext {
                            events: &[],
                            transport: ctx.transport,
                        },
                    );
                }
            }
        }
    }
    fn reset(&mut self) {
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.reset(p.modulated());
        }
        self.cached = [f32::NAN; 32];
        self.delay.fill(0.0);
        self.cursor = 0;
        match &mut self.processor {
            Processor::Advanced(p) => p.reset(),
            Processor::Eq(p) => p.reset(),
            Processor::Compressor(p) => p.reset(),
            Processor::Gate(p) => p.reset(),
            Processor::Chorus(p) => p.reset(),
            Processor::Reverb(p) => p.reset(),
            Processor::Limiter(p) => p.reset(),
            Processor::Drive(p) => p.reset(),
            _ => {}
        }
    }
    fn latency_frames(&self) -> usize {
        if let Processor::Advanced(p) = &self.processor {
            p.latency_frames()
        } else if matches!(self.processor, Processor::Drive(_)) {
            31
        } else {
            0
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_gain_is_smoothed_and_partition_independent() {
        let render = |block: usize| {
            let mut e = AutomatedEffect::new(EffectKind::Gain, "amp").unwrap();
            e.prepare(ProcessSpec {
                sample_rate: 1000.0,
                max_block_frames: 64,
                channels: 1,
            })
            .unwrap();
            e.parameters()[0].set(0.0).unwrap();
            let mut out = vec![1.0; 32];
            for b in out.chunks_mut(block) {
                e.process(
                    &mut AudioBuffer::new(b, 1).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            out
        };
        let a = render(32);
        assert_eq!(a, render(3));
        assert!((a[0] - 0.9).abs() < 1e-6);
        assert_eq!(a[9], 0.0);
    }
}
