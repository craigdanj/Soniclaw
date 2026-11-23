//! Linked peak/RMS dynamics with a bounded external key, soft knee, hold and lookahead.
use crate::precision::Sample;
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Compress,
    Expand,
    Upward,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Detector {
    Peak,
    Rms { window_frames: usize },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_ms: f32,
    pub maximum_gain_db: f32,
    pub makeup_db: f32,
    pub mix: f32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            threshold_db: -18.,
            ratio: 4.,
            knee_db: 6.,
            attack_ms: 5.,
            release_ms: 100.,
            hold_ms: 0.,
            maximum_gain_db: 24.,
            makeup_db: 0.,
            mix: 1.,
        }
    }
}
impl Controls {
    fn validate(self) -> Result<(), Error> {
        if [
            (self.threshold_db, -80., 0.),
            (self.ratio, 1., 20.),
            (self.knee_db, 0., 24.),
            (self.attack_ms, 0., 500.),
            (self.release_ms, 0., 5000.),
            (self.hold_ms, 0., 1000.),
            (self.maximum_gain_db, 0., 36.),
            (self.makeup_db, -24., 24.),
            (self.mix, 0., 1.),
        ]
        .iter()
        .any(|(x, a, b)| !x.is_finite() || x < a || x > b)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub mode: Mode,
    pub detector: Detector,
    pub key_highpass_hz: f32,
    pub lookahead_ms: f32,
    pub controls: Controls,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            mode: Mode::Compress,
            detector: Detector::Peak,
            key_highpass_hz: 0.,
            lookahead_ms: 2.,
            controls: Default::default(),
        }
    }
}
impl Config {
    fn validate(self) -> Result<(), Error> {
        self.controls.validate()?;
        if self.schema != 1
            || !self.key_highpass_hz.is_finite()
            || !(self.key_highpass_hz == 0. || (20. ..=2000.).contains(&self.key_highpass_hz))
            || !self.lookahead_ms.is_finite()
            || !(0. ..=20.).contains(&self.lookahead_ms)
            || matches!(self.detector,Detector::Rms{window_frames}if !(1..=4096).contains(&window_frames))
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Telemetry {
    pub detector_db: f32,
    pub applied_gain_db: f32,
}
fn excess(x: f64, k: f64) -> f64 {
    if k == 0. {
        x.max(0.)
    } else if x <= -k * 0.5 {
        0.
    } else if x >= k * 0.5 {
        x
    } else {
        (x + k * 0.5).powi(2) / (2. * k)
    }
}
fn target(level: f64, mode: Mode, c: Controls) -> f64 {
    let over = match mode {
        Mode::Compress => {
            (1. / c.ratio as f64 - 1.) * excess(level - c.threshold_db as f64, c.knee_db as f64)
        }
        Mode::Expand => {
            -(c.ratio as f64 - 1.) * excess(c.threshold_db as f64 - level, c.knee_db as f64)
        }
        Mode::Upward => {
            (1. - 1. / c.ratio as f64) * excess(c.threshold_db as f64 - level, c.knee_db as f64)
        }
    };
    over.clamp(-c.maximum_gain_db as f64, c.maximum_gain_db as f64)
}
pub struct Dynamics {
    config: Config,
    spec: Option<ProcessSpec>,
    delay: Vec<f64>,
    cursor: usize,
    latency: usize,
    rms: Vec<f64>,
    rms_cursor: usize,
    sum: f64,
    hpx: [f64; 2],
    hpy: [f64; 2],
    gain: f64,
    held: f64,
    hold: usize,
    telemetry: Telemetry,
}
impl Dynamics {
    pub fn new(config: Config) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            config,
            spec: None,
            delay: vec![],
            cursor: 0,
            latency: 0,
            rms: vec![],
            rms_cursor: 0,
            sum: 0.,
            hpx: [0.; 2],
            hpy: [0.; 2],
            gain: 0.,
            held: 0.,
            hold: 0,
            telemetry: Telemetry::default(),
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        controls.validate()?;
        self.config.controls = controls;
        Ok(())
    }
    pub fn telemetry(&self) -> Telemetry {
        self.telemetry
    }
    /// Interleaved mono/stereo key, exact main-frame length. Rejection occurs before state advancement.
    pub fn process_keyed(
        &mut self,
        b: &mut AudioBuffer<'_>,
        key: &[f32],
        key_channels: usize,
    ) -> Result<(), Error> {
        self.check(b.frames(), b.channels())?;
        self.check_key(b.frames(), key, key_channels)?;
        self.run(b.samples_mut(), Some((key, key_channels)));
        Ok(())
    }
    fn check_key<T: Sample>(&self, frames: usize, key: &[T], channels: usize) -> Result<(), Error> {
        if !(1..=2).contains(&channels)
            || key.len() != frames * channels
            || key
                .iter()
                .any(|sample| !(-16. ..=16.).contains(&sample.to_f64()))
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
    fn check(&self, frames: usize, channels: usize) -> Result<(), Error> {
        let s = self.spec.ok_or(Error::NotPrepared)?;
        if channels != s.channels || frames > s.max_block_frames {
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
    fn run<T: Sample>(&mut self, samples: &mut [T], key: Option<(&[T], usize)>) {
        let s = self.spec.expect("prepared");
        let c = self.config.controls;
        let alpha = |ms: f32| {
            if ms == 0. {
                1.
            } else {
                1. - (-1. / (s.sample_rate * ms as f64 * 0.001)).exp()
            }
        };
        let attack = alpha(c.attack_ms);
        let release = alpha(c.release_ms);
        let pole =
            (-std::f64::consts::TAU * self.config.key_highpass_hz as f64 / s.sample_rate).exp();
        let hold_frames = (c.hold_ms as f64 * s.sample_rate * 0.001).ceil() as usize;
        for (i, frame) in samples.chunks_exact_mut(s.channels).enumerate() {
            let mut linked = 0f64;
            let key_channels = key.map_or(s.channels, |(_, ch)| ch);
            for ch in [0, 1].into_iter().take(key_channels) {
                let raw = key
                    .map_or(frame[ch.min(s.channels - 1)], |(k, kch)| k[i * kch + ch])
                    .to_f64();
                let x = if raw.is_finite() {
                    raw.clamp(-16., 16.)
                } else {
                    0.
                };
                let hp = if self.config.key_highpass_hz == 0. {
                    x
                } else {
                    let y = pole * (self.hpy[ch] + x - self.hpx[ch]);
                    self.hpx[ch] = x;
                    self.hpy[ch] = y;
                    y
                };
                linked = linked.max(hp.abs());
            }
            let level = match self.config.detector {
                Detector::Peak => linked,
                Detector::Rms { .. } => {
                    let e = linked * linked;
                    self.sum += e - self.rms[self.rms_cursor];
                    self.rms[self.rms_cursor] = e;
                    self.rms_cursor = (self.rms_cursor + 1) % self.rms.len();
                    (self.sum.max(0.) / self.rms.len() as f64).sqrt()
                }
            };
            let db = 20. * level.max(1e-6).log10();
            let desired = target(db, self.config.mode, c);
            if desired.abs() >= self.held.abs() {
                self.held = desired;
                self.hold = hold_frames;
            } else if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.held = desired;
            }
            let rate = if self.held.abs() > self.gain.abs() {
                attack
            } else {
                release
            };
            self.gain += rate * (self.held - self.gain);
            let gain = 10f64.powf((self.gain + c.makeup_db as f64) / 20.);
            for x in frame {
                let sample = x.to_f64();
                let input = if sample.is_finite() {
                    sample.clamp(-16., 16.)
                } else {
                    0.
                };
                let dry = if self.delay.is_empty() {
                    input
                } else {
                    let delayed = self.delay[self.cursor];
                    self.delay[self.cursor] = input;
                    self.cursor = (self.cursor + 1) % self.delay.len();
                    delayed
                };
                *x = T::from_f64(dry * (1. + c.mix as f64 * (gain - 1.)));
            }
            self.telemetry = Telemetry {
                detector_db: db as f32,
                applied_gain_db: (self.gain + c.makeup_db as f64) as f32,
            };
        }
    }
}
impl AudioProcessor for Dynamics {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if !(8000. ..=192000.).contains(&s.sample_rate)
            || s.channels > 2
            || s.max_block_frames > 8192
        {
            return Err(Error::InvalidSpec);
        }
        self.latency = (self.config.lookahead_ms as f64 * s.sample_rate * 0.001).ceil() as usize;
        self.delay = vec![0.; self.latency * s.channels];
        self.rms = vec![
            0.;
            match self.config.detector {
                Detector::Peak => 1,
                Detector::Rms { window_frames } => window_frames,
            }
        ];
        self.spec = Some(s);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.check(b.frames(), b.channels()).is_err() {
            b.samples_mut().fill(0.);
            return;
        }
        self.run(b.samples_mut(), None);
    }
    fn reset(&mut self) {
        self.delay.fill(0.);
        self.rms.fill(0.);
        self.cursor = 0;
        self.rms_cursor = 0;
        self.sum = 0.;
        self.hpx = [0.; 2];
        self.hpy = [0.; 2];
        self.gain = 0.;
        self.held = 0.;
        self.hold = 0;
        self.telemetry = Telemetry::default();
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
/// Linked mono/stereo dynamics with a native f64 signal path and sidechain.
pub struct Dynamics64(Dynamics);
impl Dynamics64 {
    pub fn new(config: Config) -> Result<Self, Error> {
        Dynamics::new(config).map(Self)
    }
    pub fn config(&self) -> Config {
        self.0.config()
    }
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        self.0.set_controls(controls)
    }
    pub fn telemetry(&self) -> Telemetry {
        self.0.telemetry()
    }
    pub fn process_keyed(
        &mut self,
        buffer: &mut AudioBuffer64<'_>,
        key: &[f64],
        key_channels: usize,
    ) -> Result<(), Error> {
        self.0.check(buffer.frames(), buffer.channels())?;
        self.0.check_key(buffer.frames(), key, key_channels)?;
        self.0.run(buffer.samples_mut(), Some((key, key_channels)));
        Ok(())
    }
}
impl AudioProcessor64 for Dynamics64 {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.0.prepare(spec)
    }
    fn process(&mut self, buffer: &mut AudioBuffer64<'_>, _: &ProcessContext<'_>) {
        if self.0.check(buffer.frames(), buffer.channels()).is_err() {
            buffer.clear();
            return;
        }
        self.0.run(buffer.samples_mut(), None);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_frames(&self) -> usize {
        self.0.latency_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_modes_knee_and_limits() {
        let c = Controls {
            knee_db: 0.,
            ..Default::default()
        };
        assert!((target(-6., Mode::Compress, c) + 9.).abs() < 1e-6);
        assert!((target(-24., Mode::Expand, c) + 18.).abs() < 1e-6);
        assert!((target(-30., Mode::Upward, c) - 9.).abs() < 1e-6);
        let c = Controls::default();
        for level in -1000..1000 {
            let a = target(level as f64 * 0.1, Mode::Compress, c);
            let b = target(level as f64 * 0.1 + 1e-5, Mode::Compress, c);
            assert!((a - b).abs() < 1e-4);
        }
    }
    #[test]
    fn keyed_partition_latency_and_invalid_rejection() {
        let render = |block: usize| {
            let mut p = Dynamics::new(Config {
                detector: Detector::Rms { window_frames: 32 },
                controls: Controls {
                    attack_ms: 0.,
                    release_ms: 0.,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 2,
            })
            .unwrap();
            assert_eq!(p.latency_frames(), 96);
            let mut pcm = vec![0.25; 2048];
            let key = vec![1.; 1024];
            for (i, b) in pcm.chunks_mut(block * 2).enumerate() {
                let n = b.len() / 2;
                let mut buffer = AudioBuffer::new(b, 2).unwrap();
                assert!(p.process_keyed(&mut buffer, &[], 1).is_err());
                assert_eq!(
                    allocation_counter::measure(|| p
                        .process_keyed(&mut buffer, &key[i * block..i * block + n], 1)
                        .unwrap())
                    .count_total,
                    0
                );
            }
            assert!(pcm[..192].iter().all(|x| *x == 0.));
            assert!(pcm[400] < 0.1);
            for f in pcm.as_chunks::<2>().0 {
                assert_eq!(f[0], f[1]);
            }
            pcm
        };
        assert_eq!(render(128), render(1));
    }
}
