//! Creative nonlinear TPT, damped fractional comb and parallel vowel-resonator bank.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    LowPass,
    BandPass,
    HighPass,
    Notch,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NonlinearConfig {
    pub schema: u32,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub drive: f32,
    pub mode: Mode,
}
impl Default for NonlinearConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            cutoff_hz: 1000.,
            resonance: 0.2,
            drive: 1.,
            mode: Mode::LowPass,
        }
    }
}
impl NonlinearConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !self.cutoff_hz.is_finite()
            || !(10.0..=20000.).contains(&self.cutoff_hz)
            || !self.resonance.is_finite()
            || !(0.0..=0.98).contains(&self.resonance)
            || !self.drive.is_finite()
            || !(0.0..=16.).contains(&self.drive)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
pub struct NonlinearSvf {
    config: NonlinearConfig,
    spec: Option<ProcessSpec>,
    states: Vec<[f64; 2]>,
    g: f64,
    k: f64,
}
impl NonlinearSvf {
    pub fn new(c: NonlinearConfig) -> Result<Self, Error> {
        c.validate()?;
        Ok(Self {
            config: c,
            spec: None,
            states: vec![],
            g: 0.,
            k: 1.,
        })
    }
    pub fn set(&mut self, c: NonlinearConfig) -> Result<(), Error> {
        c.validate()?;
        self.config = c;
        if let Some(s) = self.spec {
            self.g = (std::f64::consts::PI * (c.cutoff_hz as f64).min(s.sample_rate * 0.45)
                / s.sample_rate)
                .tan();
        }
        self.k = 2. * (1. - c.resonance as f64) + 0.02;
        Ok(())
    }
}
impl AudioProcessor for NonlinearSvf {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.spec = Some(s);
        self.states = vec![[0.; 2]; s.channels];
        self.set(self.config)
    }
    fn reset(&mut self) {
        self.states.fill([0.; 2]);
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if !valid(b, s) {
            b.clear();
            return;
        }
        let g = self.g;
        let k = self.k;
        let drive = self.config.drive as f64;
        let normalizer = if drive == 0. { 1. } else { drive.tanh() };
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            for (x, z) in frame.iter_mut().zip(&mut self.states) {
                let input = if drive == 0. {
                    *x as f64
                } else {
                    (*x as f64 * drive).tanh() / normalizer
                };
                let rhs = z[0] + g * (input - z[1]);
                let mut band = rhs / (1. + g * (g + k));
                if drive > 0. {
                    for _ in 0..6 {
                        let t = (drive * band).tanh();
                        let f = (1. + g * g) * band + g * k * t / drive - rhs;
                        band -= f / (1. + g * g + g * k * (1. - t * t));
                    }
                }
                let low = z[1] + g * band;
                let damping = if drive == 0. {
                    band
                } else {
                    (drive * band).tanh() / drive
                };
                let high = input - k * damping - low;
                z[0] = bounded(2. * band - z[0]);
                z[1] = bounded(2. * low - z[1]);
                *x = bounded(match self.config.mode {
                    Mode::LowPass => low,
                    Mode::BandPass => band,
                    Mode::HighPass => high,
                    Mode::Notch => low + high,
                }) as f32;
            }
        }
    }
}
fn bounded(v: f64) -> f64 {
    if v.is_finite() {
        if v.abs() < 1e-25 {
            0.
        } else {
            v.clamp(-16., 16.)
        }
    } else {
        0.
    }
}
fn valid(b: &AudioBuffer<'_>, s: ProcessSpec) -> bool {
    b.channels() == s.channels
        && b.frames() <= s.max_block_frames
        && b.samples().iter().all(|x| x.is_finite())
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombConfig {
    pub schema: u32,
    pub frequency_hz: f32,
    pub feedback: f32,
    pub damping_hz: f32,
    pub mix: f32,
}
impl Default for CombConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            frequency_hz: 440.,
            feedback: 0.7,
            damping_hz: 6000.,
            mix: 0.5,
        }
    }
}
impl CombConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !self.frequency_hz.is_finite()
            || !(20.0..=20000.).contains(&self.frequency_hz)
            || !self.feedback.is_finite()
            || !(-0.98..=0.98).contains(&self.feedback)
            || !self.damping_hz.is_finite()
            || !(0.0..=20000.).contains(&self.damping_hz)
            || !self.mix.is_finite()
            || !(0.0..=1.).contains(&self.mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
pub struct CombFilter {
    config: CombConfig,
    spec: Option<ProcessSpec>,
    ring: Vec<f32>,
    low: Vec<f32>,
    cursor: usize,
    frames: usize,
    delay: f64,
    target: f64,
    damping: f64,
    slew: f64,
}
impl CombFilter {
    pub fn new(c: CombConfig) -> Result<Self, Error> {
        c.validate()?;
        Ok(Self {
            config: c,
            spec: None,
            ring: vec![],
            low: vec![],
            cursor: 0,
            frames: 0,
            delay: 0.,
            target: 0.,
            damping: 1.,
            slew: 1.,
        })
    }
    pub fn set(&mut self, c: CombConfig) -> Result<(), Error> {
        c.validate()?;
        self.config = c;
        if let Some(s) = self.spec {
            self.target = (s.sample_rate / c.frequency_hz as f64).max(2.);
            self.damping = if c.damping_hz == 0. {
                1.
            } else {
                -(-std::f64::consts::TAU * c.damping_hz as f64 / s.sample_rate).exp_m1()
            };
        }
        Ok(())
    }
}
impl AudioProcessor for CombFilter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        let frames = (s.sample_rate / 20.).ceil() as usize + 2;
        let count = frames.checked_mul(s.channels).ok_or(Error::InvalidSpec)?;
        if count > 4 * 1024 * 1024 {
            return Err(Error::InvalidSpec);
        }
        self.spec = Some(s);
        self.frames = frames;
        self.ring = vec![0.; count];
        self.low = vec![0.; s.channels];
        self.slew = -(-1. / (s.sample_rate * 0.01)).exp_m1();
        self.set(self.config)?;
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.ring.fill(0.);
        self.low.fill(0.);
        self.cursor = 0;
        self.delay = self.target;
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if !valid(b, s) {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            self.delay += self.slew * (self.target - self.delay);
            let whole = self.delay.floor() as usize;
            let fraction = (self.delay - whole as f64) as f32;
            let a = (self.cursor + self.frames - whole) % self.frames;
            let d = (a + self.frames - 1) % self.frames;
            for (c, x) in frame.iter_mut().enumerate() {
                let value = self.ring[a * s.channels + c] * (1. - fraction)
                    + self.ring[d * s.channels + c] * fraction;
                self.low[c] += (self.damping as f32) * (value - self.low[c]);
                self.ring[self.cursor * s.channels + c] =
                    (*x + self.config.feedback * self.low[c]).clamp(-16., 16.);
                *x = (1. - self.config.mix) * *x + self.config.mix * value;
            }
            self.cursor = (self.cursor + 1) % self.frames;
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormantConfig {
    pub schema: u32,
    pub vowel: f32,
    pub shift_semitones: f32,
    pub mix: f32,
}
impl Default for FormantConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            vowel: 0.,
            shift_semitones: 0.,
            mix: 1.,
        }
    }
}
impl FormantConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !self.vowel.is_finite()
            || !(0.0..=4.).contains(&self.vowel)
            || !self.shift_semitones.is_finite()
            || !(-24.0..=24.).contains(&self.shift_semitones)
            || !self.mix.is_finite()
            || !(0.0..=1.).contains(&self.mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
pub struct FormantFilter {
    config: FormantConfig,
    spec: Option<ProcessSpec>,
    states: Vec<[[f64; 2]; 3]>,
    g: [f64; 3],
    target: [f64; 3],
    k: [f64; 3],
    target_k: [f64; 3],
    slew: f64,
}
impl FormantFilter {
    pub fn new(c: FormantConfig) -> Result<Self, Error> {
        c.validate()?;
        Ok(Self {
            config: c,
            spec: None,
            states: vec![],
            g: [0.; 3],
            target: [0.; 3],
            k: [0.; 3],
            target_k: [0.; 3],
            slew: 1.,
        })
    }
    pub fn set(&mut self, c: FormantConfig) -> Result<(), Error> {
        c.validate()?;
        self.config = c;
        if let Some(s) = self.spec {
            let vowels = [
                [800., 1150., 2900.],
                [400., 1700., 2600.],
                [350., 2000., 2800.],
                [450., 800., 2830.],
                [325., 700., 2530.],
            ];
            let a = c.vowel.floor() as usize;
            let b = (a + 1).min(4);
            let t = c.vowel as f64 - a as f64;
            let shift = 2f64.powf(c.shift_semitones as f64 / 12.);
            for i in 0..3 {
                let hz =
                    ((vowels[a][i] * (1. - t) + vowels[b][i] * t) * shift).min(s.sample_rate * 0.4);
                self.target[i] = (std::f64::consts::PI * hz / s.sample_rate).tan();
                self.target_k[i] = ([80., 90., 120.][i] / hz).clamp(0.01, 1.);
            }
        }
        Ok(())
    }
}
impl AudioProcessor for FormantFilter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.spec = Some(s);
        self.states = vec![[[0.; 2]; 3]; s.channels];
        self.slew = -(-1. / (s.sample_rate * 0.02)).exp_m1();
        self.set(self.config)?;
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.states.fill([[0.; 2]; 3]);
        self.g = self.target;
        self.k = self.target_k;
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if !valid(b, s) {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            for i in 0..3 {
                self.g[i] += self.slew * (self.target[i] - self.g[i]);
                self.k[i] += self.slew * (self.target_k[i] - self.k[i]);
            }
            for (x, states) in frame.iter_mut().zip(&mut self.states) {
                let input = *x as f64;
                let mut wet = 0.;
                for (i, z) in states.iter_mut().enumerate() {
                    let g = self.g[i];
                    let k = self.k[i];
                    let a = 1. / (1. + g * (g + k));
                    let band = a * z[0] + g * a * (input - z[1]);
                    let low = z[1] + g * band;
                    z[0] = bounded(2. * band - z[0]);
                    z[1] = bounded(2. * low - z[1]);
                    wet += [1., 0.7, 0.4][i] * k * band;
                }
                *x = bounded((1. - self.config.mix as f64) * input + self.config.mix as f64 * wet)
                    as f32;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ProcessContext<'static> {
        ProcessContext {
            events: &[],
            transport: Transport::default(),
        }
    }
    #[test]
    fn comb_impulse_tuning_decay_partition_and_allocations() {
        let cfg = CombConfig {
            frequency_hz: 1000.,
            feedback: 0.5,
            damping_hz: 0.,
            mix: 1.,
            ..Default::default()
        };
        let render = |n: usize| {
            let mut p = CombFilter::new(cfg).unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 127,
                channels: 1,
            })
            .unwrap();
            let mut data = vec![0.; 512];
            data[0] = 1.;
            for b in data.chunks_mut(n) {
                assert_eq!(
                    allocation_counter::measure(
                        || p.process(&mut AudioBuffer::new(b, 1).unwrap(), &ctx())
                    )
                    .count_total,
                    0
                );
            }
            assert_eq!(data[48], 1.);
            assert_eq!(data[96], 0.5);
            p.reset();
            data
        };
        assert_eq!(render(127), render(1));
    }
    #[test]
    fn nonlinear_filter_sweeps_and_morph_stay_finite() {
        let s = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 2,
        };
        let mut p = NonlinearSvf::new(Default::default()).unwrap();
        let mut f = FormantFilter::new(Default::default()).unwrap();
        p.prepare(s).unwrap();
        f.prepare(s).unwrap();
        for i in 0..200 {
            p.set(NonlinearConfig {
                cutoff_hz: 20. + i as f32 * 99.,
                resonance: 0.98,
                drive: 16.,
                mode: Mode::BandPass,
                ..Default::default()
            })
            .unwrap();
            f.set(FormantConfig {
                vowel: (i % 40) as f32 * 0.1,
                ..Default::default()
            })
            .unwrap();
            let mut data = [0.5; 256];
            assert_eq!(
                allocation_counter::measure(|| {
                    p.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx());
                    f.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx());
                })
                .count_total,
                0
            );
            assert!(data.iter().all(|x| x.is_finite() && x.abs() <= 16.));
        }
        assert!(p
            .set(NonlinearConfig {
                cutoff_hz: f32::NAN,
                ..Default::default()
            })
            .is_err());
    }
}
#[cfg(test)]
mod response_tests {
    use super::*;
    #[test]
    fn linear_limit_has_expected_cutoff_gain() {
        let mut p = NonlinearSvf::new(NonlinearConfig {
            cutoff_hz: 1000.,
            resonance: 0.30289322,
            drive: 0.,
            mode: Mode::LowPass,
            ..Default::default()
        })
        .unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 8192,
            channels: 1,
        })
        .unwrap();
        let mut pcm: Vec<f32> = (0..8192)
            .map(|i| 0.1 * (std::f64::consts::TAU * 1000. * i as f64 / 48000.).sin() as f32)
            .collect();
        p.process(
            &mut AudioBuffer::new(&mut pcm, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        let rms = (pcm[3392..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 4800.).sqrt();
        assert!((rms - 0.05).abs() < 0.0001, "rms {rms}");
    }
    #[test]
    fn formant_partition_channel_isolation_and_reset() {
        let render = |block: usize| {
            let mut p = FormantFilter::new(Default::default()).unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 127,
                channels: 2,
            })
            .unwrap();
            let mut data = vec![0.; 4096];
            data[0] = 1.;
            for b in data.chunks_mut(block * 2) {
                p.process(
                    &mut AudioBuffer::new(b, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            assert!(data.iter().step_by(2).any(|x| x.abs() > 0.01));
            assert!(data.iter().skip(1).step_by(2).all(|x| *x == 0.));
            p.reset();
            data
        };
        assert_eq!(render(127), render(1));
    }
}
#[cfg(test)]
mod continuity_tests {
    use super::*;
    #[test]
    fn near_zero_drive_matches_linear_limit() {
        let run = |drive| {
            let mut p = NonlinearSvf::new(NonlinearConfig {
                drive,
                ..Default::default()
            })
            .unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 1,
            })
            .unwrap();
            let mut b = [0.; 128];
            b[0] = 0.1;
            p.process(
                &mut AudioBuffer::new(&mut b, 1).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
            b
        };
        let a = run(0.);
        let b = run(1e-6);
        assert!(a.iter().zip(b).all(|(a, b)| (*a - b).abs() < 1e-5));
    }
}
