//! Bounded note-local modulation, independent of the global atomic matrix.
use crate::{modulation::LfoShape, Adsr, AdsrSettings, ExpressiveSynth};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSource {
    Envelope1,
    Envelope2,
    Lfo1,
    Lfo2,
    Velocity,
    Key,
    Pressure,
    Timbre,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceDestination {
    Pitch,
    Cutoff,
    Level,
    Pan,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRoute {
    pub source: VoiceSource,
    pub destination: VoiceDestination,
    pub amount: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceLfo {
    pub shape: LfoShape,
    pub rate_hz: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceModulationConfig {
    pub envelopes: [AdsrSettings; 2],
    pub lfos: [VoiceLfo; 2],
    pub routes: Vec<VoiceRoute>,
}
impl Default for VoiceModulationConfig {
    fn default() -> Self {
        Self {
            envelopes: [
                AdsrSettings::default(),
                AdsrSettings {
                    attack: 0.25,
                    decay: 0.3,
                    sustain: 0.4,
                    release: 0.5,
                },
            ],
            lfos: [
                VoiceLfo {
                    shape: LfoShape::Sine,
                    rate_hz: 4.0,
                },
                VoiceLfo {
                    shape: LfoShape::Triangle,
                    rate_hz: 0.7,
                },
            ],
            routes: vec![],
        }
    }
}
impl VoiceModulationConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.routes.len() > 32
            || self
                .routes
                .iter()
                .any(|r| !r.amount.is_finite() || !(-1.0..=1.0).contains(&r.amount))
            || self
                .lfos
                .iter()
                .any(|l| !l.rate_hz.is_finite() || !(0.01..=40.0).contains(&l.rate_hz))
        {
            return Err(Error::InvalidParameter);
        }
        for e in self.envelopes {
            Adsr::default().configure(e)?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct VoiceModState {
    envelopes: [Adsr; 2],
    phase: [f64; 2],
    random: [u32; 2],
    held: [f32; 2],
}
impl Default for VoiceModState {
    fn default() -> Self {
        Self {
            envelopes: [Adsr::default(); 2],
            phase: [0.0; 2],
            random: [0x69726f6e, 0x736f6e69],
            held: [0.0; 2],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct VoiceOffsets {
    pub pitch_semitones: f32,
    pub cutoff_octaves: f32,
    pub level: f32,
    pub pan: f32,
}
impl Default for VoiceOffsets {
    fn default() -> Self {
        Self {
            pitch_semitones: 0.0,
            cutoff_octaves: 0.0,
            level: 1.0,
            pan: 0.0,
        }
    }
}
impl VoiceModState {
    pub fn start(&mut self, cfg: &VoiceModulationConfig, seed: u32) {
        *self = Self::default();
        for i in 0..2 {
            self.random[i] = (self.random[i] ^ seed.wrapping_mul(0x9e3779b9)).max(1);
            let _ = self.envelopes[i].configure(cfg.envelopes[i]);
            self.envelopes[i].note_on();
        }
    }
    pub fn release(&mut self, rate: f32) {
        for e in &mut self.envelopes {
            e.note_off(rate);
        }
    }
    pub fn tick(
        &mut self,
        cfg: &VoiceModulationConfig,
        rate: f32,
        note: u8,
        velocity: f32,
        pressure: f32,
        timbre: f32,
    ) -> VoiceOffsets {
        if cfg.routes.is_empty() {
            return VoiceOffsets::default();
        }
        let mut values = [0.0; 8];
        for i in 0..2 {
            values[i] = self.envelopes[i].next_sample(rate);
            let t = self.phase[i] as f32;
            values[i + 2] = match cfg.lfos[i].shape {
                LfoShape::Sine => (std::f32::consts::TAU * t).sin(),
                LfoShape::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                LfoShape::Saw => 2.0 * t - 1.0,
                LfoShape::Square => {
                    if t < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                LfoShape::SampleHold => self.held[i],
            };
            self.phase[i] += cfg.lfos[i].rate_hz as f64 / rate as f64;
            if self.phase[i] >= 1.0 {
                self.phase[i] = self.phase[i].rem_euclid(1.0);
                let r = &mut self.random[i];
                *r ^= *r << 13;
                *r ^= *r >> 17;
                *r ^= *r << 5;
                self.held[i] = *r as f32 / u32::MAX as f32 * 2.0 - 1.0;
            }
        }
        values[4] = velocity;
        values[5] = (note as f32 - 60.0) / 60.0;
        values[6] = pressure;
        values[7] = timbre;
        let mut sum = [0.0_f32; 4];
        for r in &cfg.routes {
            sum[r.destination as usize] += values[r.source as usize] * r.amount;
        }
        VoiceOffsets {
            pitch_semitones: sum[0].clamp(-1.0, 1.0) * 24.0,
            cutoff_octaves: sum[1].clamp(-1.0, 1.0) * 5.0,
            level: (1.0 + sum[2]).clamp(0.0, 4.0),
            pan: sum[3].clamp(-1.0, 1.0),
        }
    }
}
pub struct PolyModSynth<const N: usize = 32> {
    pub synth: ExpressiveSynth<N>,
    parameters: Vec<Arc<Parameter>>,
    gain: SmoothedValue,
    prepared: bool,
}
impl<const N: usize> PolyModSynth<N> {
    pub fn new(prefix: &str, config: VoiceModulationConfig) -> Result<Self, Error> {
        config.validate()?;
        if prefix.is_empty() || N == 0 || N > 128 {
            return Err(Error::InvalidParameter);
        }
        let mut synth = ExpressiveSynth::default();
        synth.voice_modulation = config;
        let parameters = [
            ("gain", 0.0, 1.0, 0.6, ""),
            ("cutoff", 20.0, 20000.0, 3000.0, "Hz"),
            ("pressure", 0.0, 2.0, 0.5, ""),
            ("timbre", 0.0, 1.0, 0.7, ""),
        ]
        .into_iter()
        .map(|(key, min, max, default, unit)| {
            Ok(Arc::new(Parameter::new(ParameterInfo {
                id: &format!("{prefix}.{key}"),
                name: key,
                min,
                max,
                default,
                unit,
            })?))
        })
        .collect::<Result<_, Error>>()?;
        Ok(Self {
            synth,
            parameters,
            gain: SmoothedValue::new(0.6, 480),
            prepared: false,
        })
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn reuse_parameters(&mut self, old: &[Arc<Parameter>]) -> Result<(), Error> {
        for p in &mut self.parameters {
            if let Some(o) = old.iter().find(|q| q.info().id == p.info().id) {
                if o.info().min != p.info().min || o.info().max != p.info().max {
                    return Err(Error::InvalidParameter);
                }
                *p = o.clone();
            } else if !old.is_empty() {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
}
impl<const N: usize> AudioProcessor for PolyModSynth<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.prepared = false;
        if !(8000.0..=192000.0).contains(&s.sample_rate) || s.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        self.synth.prepare(s)?;
        self.gain = SmoothedValue::new(
            self.parameters[0].modulated(),
            (s.sample_rate * 0.01).ceil() as usize,
        );
        self.prepared = true;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if !self.prepared {
            b.clear();
            return;
        }
        self.synth.cutoff_hz = Some(self.parameters[1].modulated());
        self.synth.pressure_gain = self.parameters[2].modulated();
        self.synth.timbre_depth = self.parameters[3].modulated();
        self.gain.set_target(self.parameters[0].modulated());
        self.synth.process(b, c);
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            let gain = self.gain.next_value();
            for x in f {
                *x *= gain;
            }
        }
    }
    fn reset(&mut self) {
        self.synth.reset();
        self.gain.reset(self.parameters[0].modulated());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voices_have_independent_lfos_envelopes_and_release() {
        let cfg = VoiceModulationConfig {
            routes: vec![
                VoiceRoute {
                    source: VoiceSource::Envelope1,
                    destination: VoiceDestination::Level,
                    amount: -1.0,
                },
                VoiceRoute {
                    source: VoiceSource::Lfo1,
                    destination: VoiceDestination::Pitch,
                    amount: 0.1,
                },
            ],
            ..Default::default()
        };
        let (mut a, mut b) = (VoiceModState::default(), VoiceModState::default());
        a.start(&cfg, 1);
        b.start(&cfg, 2);
        for _ in 0..2000 {
            a.tick(&cfg, 48000.0, 60, 1.0, 0.0, 0.5);
        }
        let x = a.tick(&cfg, 48000.0, 60, 1.0, 0.0, 0.5);
        let y = b.tick(&cfg, 48000.0, 60, 1.0, 0.0, 0.5);
        assert!((x.level - y.level).abs() > 0.2);
        assert!((x.pitch_semitones - y.pitch_semitones).abs() > 0.1);
        a.release(48000.0);
        for _ in 0..24000 {
            a.tick(&cfg, 48000.0, 60, 1.0, 0.0, 0.5);
        }
        assert!(a.tick(&cfg, 48000.0, 60, 1.0, 0.0, 0.5).level > 0.99);
    }
}
