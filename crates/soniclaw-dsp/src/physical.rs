//! Fractional-delay strings and damped modal resonators; independent voice building blocks.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StringConfig {
    pub schema: u32,
    pub frequency: f32,
    pub decay_seconds: f32,
    pub brightness: f32,
    pub dispersion: f32,
    pub bow_pressure: f32,
    pub bow_velocity: f32,
}
impl Default for StringConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            frequency: 220.,
            decay_seconds: 3.,
            brightness: 0.6,
            dispersion: 0.3,
            bow_pressure: 0.,
            bow_velocity: 0.2,
        }
    }
}
impl StringConfig {
    fn validate(self) -> Result<(), Error> {
        if self.schema != 1
            || [
                (self.frequency, 20., 2000.),
                (self.decay_seconds, 0.1, 30.),
                (self.brightness, 0., 1.),
                (self.dispersion, 0., 0.95),
                (self.bow_pressure, 0., 1.),
                (self.bow_velocity, -1., 1.),
            ]
            .iter()
            .any(|(x, a, b)| !x.is_finite() || x < a || x > b)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
pub struct WaveguideString {
    config: StringConfig,
    ring: Vec<f32>,
    cursor: usize,
    delay: f64,
    loss: f32,
    low: f32,
    apx: f32,
    apy: f32,
    spec: Option<ProcessSpec>,
}
impl WaveguideString {
    pub fn new(config: StringConfig) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            config,
            ring: vec![],
            cursor: 0,
            delay: 0.,
            loss: 0.,
            low: 0.,
            apx: 0.,
            apy: 0.,
            spec: None,
        })
    }
    pub fn config(&self) -> StringConfig {
        self.config
    }
    pub fn set_config(&mut self, config: StringConfig) -> Result<(), Error> {
        config.validate()?;
        if self
            .spec
            .is_some_and(|s| config.frequency as f64 >= s.sample_rate * 0.4)
        {
            return Err(Error::InvalidParameter);
        }
        self.config = config;
        if self.spec.is_some() {
            self.tune();
        }
        Ok(())
    }
    fn tune(&mut self) {
        let c = self.config;
        let sr = self.spec.expect("prepared").sample_rate;
        let alpha = 0.15 + 0.8 * c.brightness as f64;
        let ap = (1. - c.dispersion as f64) / (1. + c.dispersion as f64);
        self.delay = (sr / c.frequency as f64 - (1. - alpha) / alpha - ap).max(2.);
        self.loss = 10f32.powf(-3. / (c.frequency * c.decay_seconds));
    }
    /// Deterministic pick-position difference noise. No allocation; call only on the audio owner.
    pub fn pluck(&mut self, amplitude: f32, position: f32, seed: u32) -> Result<(), Error> {
        if self.spec.is_none() {
            return Err(Error::NotPrepared);
        }
        if !amplitude.is_finite()
            || !(0. ..=1.).contains(&amplitude)
            || !position.is_finite()
            || !(0.02..=0.98).contains(&position)
        {
            return Err(Error::InvalidParameter);
        }
        self.reset();
        let n = self.delay.ceil() as usize;
        let mut rng = seed.max(1);
        for x in &mut self.ring[..n] {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            *x = (rng as f64 / u32::MAX as f64 * 2. - 1.) as f32;
        }
        let offset = (position * n as f32).round() as usize; // difference requires original data; use a fixed stack copy, bounded by minimum frequency/rate.
        let mut copy = [0f32; 9602];
        copy[..n].copy_from_slice(&self.ring[..n]);
        for i in 0..n {
            self.ring[i] = amplitude * 0.5 * (copy[i] - copy[(i + offset) % n]);
        }
        self.cursor = n;
        Ok(())
    }
    pub fn tick(&mut self) -> f32 {
        self.tick_with_drive(0.0)
    }

    /// Inject bounded external excitation into the prepared waveguide feedback junction.
    pub fn tick_with_drive(&mut self, drive: f32) -> f32 {
        let drive = if drive.is_finite() {
            drive.clamp(-0.25, 0.25)
        } else {
            0.0
        };
        if self.spec.is_none() {
            return 0.;
        }
        let len = self.ring.len();
        let p = (self.cursor as f64 + len as f64 - self.delay).rem_euclid(len as f64);
        let a = p.floor() as usize;
        let y = self.ring[a] + (self.ring[(a + 1) % len] - self.ring[a]) * p.fract() as f32;
        let alpha = 0.15 + 0.8 * self.config.brightness;
        self.low += alpha * (y - self.low);
        let d = self.config.dispersion;
        let dispersed = d * self.low + self.apx - d * self.apy;
        self.apx = self.low;
        self.apy = dispersed;
        let bow = 0.015 * self.config.bow_pressure * ((self.config.bow_velocity - y) * 4.).tanh();
        self.ring[self.cursor] = (dispersed * self.loss + bow + drive).clamp(-2., 2.);
        self.cursor = (self.cursor + 1) % len;
        y
    }
}
fn valid(spec: ProcessSpec) -> bool {
    spec.validate().is_ok()
        && (8000. ..=192000.).contains(&spec.sample_rate)
        && spec.channels <= 2
        && spec.max_block_frames <= 8192
}
impl AudioProcessor for WaveguideString {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        if !valid(spec) || self.config.frequency as f64 >= spec.sample_rate * 0.4 {
            return Err(Error::InvalidParameter);
        }
        self.ring = vec![0.; (spec.sample_rate / 20.).ceil() as usize + 2];
        self.spec = Some(spec);
        self.tune();
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.ring.fill(0.);
        self.cursor = 0;
        self.low = 0.;
        self.apx = 0.;
        self.apy = 0.;
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(s.channels) {
            f.fill(self.tick());
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    pub frequency: f32,
    pub decay_seconds: f32,
    pub gain: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModalConfig {
    pub schema: u32,
    pub modes: Vec<Mode>,
}
impl ModalConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || self.modes.is_empty()
            || self.modes.len() > 32
            || self.modes.iter().any(|m| {
                !m.frequency.is_finite()
                    || !(20. ..=20000.).contains(&m.frequency)
                    || !m.decay_seconds.is_finite()
                    || !(0.01..=30.).contains(&m.decay_seconds)
                    || !m.gain.is_finite()
                    || m.gain.abs() > 1.
            })
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Default)]
struct Resonator {
    a: f32,
    b: f32,
    g: f32,
    y: f32,
    z: f32,
}
pub struct ModalBank {
    config: ModalConfig,
    modes: Vec<Resonator>,
    spec: Option<ProcessSpec>,
}
impl ModalBank {
    pub fn new(config: ModalConfig) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            config,
            modes: vec![],
            spec: None,
        })
    }
    pub fn config(&self) -> &ModalConfig {
        &self.config
    }
    /// Drives every mode with one excitation sample; sums with sqrt(mode-count) normalization.
    pub fn tick(&mut self, excitation: f32) -> f32 {
        if !excitation.is_finite() {
            return 0.;
        }
        let mut sum = 0.;
        for m in &mut self.modes {
            let next = m.a * m.y - m.b * m.z + m.g * excitation.clamp(-1., 1.);
            m.z = m.y;
            m.y = next;
            sum += next;
        }
        sum / (self.modes.len().max(1) as f32).sqrt()
    }
}
impl AudioProcessor for ModalBank {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if !valid(s)
            || self
                .config
                .modes
                .iter()
                .any(|m| m.frequency as f64 >= s.sample_rate * 0.4)
        {
            return Err(Error::InvalidParameter);
        }
        self.modes = self
            .config
            .modes
            .iter()
            .map(|m| {
                let r = 10f64.powf(-3. / (m.decay_seconds as f64 * s.sample_rate));
                let w = std::f64::consts::TAU * m.frequency as f64 / s.sample_rate;
                Resonator {
                    a: (2. * r * w.cos()) as f32,
                    b: (r * r) as f32,
                    g: m.gain * w.sin() as f32,
                    ..Default::default()
                }
            })
            .collect();
        self.spec = Some(s);
        Ok(())
    }
    fn reset(&mut self) {
        for m in &mut self.modes {
            m.y = 0.;
            m.z = 0.;
        }
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(s.channels) {
            let y = self.tick(f[0]);
            f.fill(y);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        }
    }
    #[test]
    fn string_partition_and_stereo_coherence() {
        let render = |block: usize| {
            let mut p = WaveguideString::new(StringConfig {
                bow_pressure: 0.3,
                ..Default::default()
            })
            .unwrap();
            p.prepare(ProcessSpec {
                channels: 2,
                ..spec()
            })
            .unwrap();
            p.pluck(0.5, 0.3, 19).unwrap();
            let mut data = vec![0.; 8192];
            for b in data.chunks_mut(block * 2) {
                p.process(
                    &mut AudioBuffer::new(b, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            for f in data.as_chunks::<2>().0 {
                assert_eq!(f[0], f[1]);
            }
            data
        };
        assert_eq!(render(128), render(1));
    }
    #[test]
    fn pluck_tuning_decay_and_reset() {
        let mut s = WaveguideString::new(Default::default()).unwrap();
        s.prepare(spec()).unwrap();
        s.pluck(0.8, 0.23, 42).unwrap();
        let info = allocation_counter::measure(|| {
            for _ in 0..128 {
                s.tick();
            }
        });
        assert_eq!(info.count_total, 0);
        s.pluck(0.8, 0.23, 42).unwrap();
        let data: Vec<_> = (0..48000).map(|_| s.tick()).collect();
        let lag = (210..230)
            .max_by(|a, b| {
                let score = |lag: usize| {
                    (2000..8000)
                        .map(|i| data[i] as f64 * data[i + lag] as f64)
                        .sum::<f64>()
                };
                score(*a).total_cmp(&score(*b))
            })
            .unwrap();
        assert!((48000. / lag as f64 - 220.).abs() < 5.);
        let energy = |a: usize| data[a..a + 4000].iter().map(|x| x * x).sum::<f32>();
        assert!(energy(40000) < energy(4000));
        s.reset();
        assert_eq!(s.tick(), 0.);
        assert!(s
            .set_config(StringConfig {
                frequency: f32::NAN,
                ..Default::default()
            })
            .is_err());
    }
    #[test]
    fn modal_impulse_has_correct_decay() {
        let mut m = ModalBank::new(ModalConfig {
            schema: 1,
            modes: vec![Mode {
                frequency: 1000.,
                decay_seconds: 0.1,
                gain: 1.,
            }],
        })
        .unwrap();
        m.prepare(spec()).unwrap();
        let mut data = vec![0.; 6000];
        data[0] = 1.;
        let ctx = ProcessContext {
            events: &[],
            transport: Transport::default(),
        };
        for b in data.chunks_mut(128) {
            assert_eq!(
                allocation_counter::measure(
                    || m.process(&mut AudioBuffer::new(b, 1).unwrap(), &ctx)
                )
                .count_total,
                0
            );
        }
        let energy = |a: usize| data[a..a + 240].iter().map(|x| x * x).sum::<f32>();
        let db = 10. * (energy(4800) / energy(0)).log10();
        assert!((db + 60.).abs() < 0.3);
    }
}
