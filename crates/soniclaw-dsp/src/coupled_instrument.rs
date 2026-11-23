//! Coupled strings driven through a common modal soundboard.
use crate::physical::{ModalBank, ModalConfig, StringConfig, WaveguideString};
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema: u32,
    pub strings: Vec<StringConfig>,
    pub body: ModalConfig,
    pub coupling: f32,
    pub body_mix: f32,
    pub hammer_ms: f32,
}
#[derive(Clone, Copy, Default)]
struct Hammer {
    amplitude: f32,
    frames: usize,
    remaining: usize,
}
impl Hammer {
    fn tick(&mut self) -> f32 {
        if self.remaining == 0 {
            return 0.;
        }
        let elapsed = self.frames - self.remaining;
        self.remaining -= 1;
        self.amplitude * (std::f32::consts::PI * (elapsed as f32 + 0.5) / self.frames as f32).sin()
    }
}
pub struct CoupledInstrument {
    settings: Settings,
    strings: Vec<WaveguideString>,
    sympathetic: Vec<WaveguideString>,
    body: ModalBank,
    hammer: Vec<Hammer>,
    last: Vec<f32>,
    bridge: f32,
    spec: Option<ProcessSpec>,
}
impl CoupledInstrument {
    pub fn new(settings: Settings) -> Result<Self, Error> {
        if settings.schema != 1
            || !(1..=12).contains(&settings.strings.len())
            || !(0. ..=0.04).contains(&settings.coupling)
            || !(0. ..=1.).contains(&settings.body_mix)
            || !(0.2..=8.).contains(&settings.hammer_ms)
        {
            return Err(Error::InvalidParameter);
        }
        let body = ModalBank::new(settings.body.clone())?;
        let strings = settings
            .strings
            .iter()
            .copied()
            .map(WaveguideString::new)
            .collect::<Result<Vec<_>, _>>()?;
        let len = strings.len();
        Ok(Self {
            settings,
            strings,
            sympathetic: Vec::new(),
            body,
            hammer: vec![Hammer::default(); len],
            last: vec![0.; len],
            bridge: 0.,
            spec: None,
        })
    }
    pub fn strike(&mut self, string: usize, velocity: f32) -> Result<(), Error> {
        let spec = self.spec.ok_or(Error::NotPrepared)?;
        if !(0. ..=1.).contains(&velocity) {
            return Err(Error::InvalidParameter);
        }
        let h = self.hammer.get_mut(string).ok_or(Error::InvalidParameter)?;
        let seconds =
            f64::from(self.settings.hammer_ms) * (1.3 - 0.7 * f64::from(velocity)) * 0.001;
        let frames = (spec.sample_rate * seconds).round().max(1.) as usize;
        *h = Hammer {
            amplitude: 0.13 * velocity.powf(1.45),
            frames,
            remaining: frames,
        };
        Ok(())
    }
    pub fn pluck(
        &mut self,
        string: usize,
        velocity: f32,
        position: f32,
        seed: u32,
    ) -> Result<(), Error> {
        if !(0. ..=1.).contains(&velocity) {
            return Err(Error::InvalidParameter);
        }
        self.strings
            .get_mut(string)
            .ok_or(Error::InvalidParameter)?
            .pluck(velocity.powf(1.3), position, seed)
    }
    pub fn string_values(&self) -> &[f32] {
        &self.last
    }
    pub fn tick(&mut self) -> f32 {
        if self.spec.is_none() {
            return 0.;
        }
        let mut dry_sum = 0.;
        for ((last, string), hammer) in self
            .last
            .iter_mut()
            .zip(&mut self.strings)
            .zip(&mut self.hammer)
        {
            *last = string.tick_with_drive(hammer.tick());
            dry_sum += *last;
        }
        let count = self.strings.len() as f32;
        let body = self.body.tick(dry_sum / count);
        self.bridge += 0.16 * (body.tanh() - self.bridge);
        let drive = self.bridge * self.settings.coupling;

        // Keep the body input on the direct path. The response bank can ring
        // alongside the struck strings without circulating energy back to the body.
        let mut string_sum = 0.;
        for (last, resonator) in self.last.iter_mut().zip(&mut self.sympathetic) {
            *last = (*last + resonator.tick_with_drive(drive)).clamp(-2., 2.);
            string_sum += *last;
        }
        (string_sum / count) * (1. - self.settings.body_mix)
            + body.clamp(-2., 2.) * self.settings.body_mix
    }
}
impl AudioProcessor for CoupledInstrument {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels > 2
            || !(8000. ..=192000.).contains(&spec.sample_rate)
            || spec.max_block_frames > 8192
        {
            return Err(Error::InvalidSpec);
        }
        let spec_mono = ProcessSpec {
            channels: 1,
            ..spec
        };
        let mut strings = self
            .settings
            .strings
            .iter()
            .copied()
            .map(WaveguideString::new)
            .collect::<Result<Vec<_>, _>>()?;
        for s in &mut strings {
            s.prepare(spec_mono)?;
        }
        let mut sympathetic = Vec::with_capacity(strings.len());
        for &recipe in &self.settings.strings {
            // The sympathetic component responds only to the shared body.
            let mut resonator = WaveguideString::new(StringConfig {
                bow_pressure: 0.,
                ..recipe
            })?;
            resonator.prepare(spec_mono)?;
            sympathetic.push(resonator);
        }
        let mut body = ModalBank::new(self.settings.body.clone())?;
        body.prepare(spec_mono)?;
        self.strings = strings;
        self.sympathetic = sympathetic;
        self.body = body;
        self.spec = Some(spec);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != spec.channels || b.frames() > spec.max_block_frames {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(spec.channels) {
            frame.fill(self.tick());
        }
    }
    fn reset(&mut self) {
        for s in self.strings.iter_mut().chain(&mut self.sympathetic) {
            s.reset();
        }
        self.body.reset();
        self.hammer.fill(Hammer::default());
        self.last.fill(0.);
        self.bridge = 0.;
    }
}
