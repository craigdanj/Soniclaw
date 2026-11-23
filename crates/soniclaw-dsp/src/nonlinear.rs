//! First-antiderivative anti-aliasing with explicit integer-delay output alignment.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    HardClip,
    Tanh,
    Cubic,
}
impl Shape {
    pub fn sample(self, x: f64) -> f64 {
        match self {
            Self::HardClip => x.clamp(-1., 1.),
            Self::Tanh => x.tanh(),
            Self::Cubic => {
                if x.abs() >= 1. {
                    x.signum()
                } else {
                    1.5 * x - 0.5 * x * x * x
                }
            }
        }
    }
    fn integral(self, x: f64) -> f64 {
        let a = x.abs();
        match self {
            Self::HardClip => {
                if a <= 1. {
                    0.5 * x * x
                } else {
                    a - 0.5
                }
            }
            Self::Tanh => a + (-2. * a).exp().ln_1p() - std::f64::consts::LN_2,
            Self::Cubic => {
                if a <= 1. {
                    0.75 * x * x - 0.125 * x.powi(4)
                } else {
                    a - 0.375
                }
            }
        }
    }
}
pub struct Adaa1 {
    shape: Shape,
    previous: f64,
}
impl Adaa1 {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            previous: 0.,
        }
    }
    pub fn reset(&mut self) {
        self.previous = 0.;
    }
    /// Half a sample of small-signal delay. This primitive is not an AudioProcessor.
    pub fn fractional_latency_frames(&self) -> f64 {
        0.5
    }
    pub fn tick(&mut self, x: f64) -> f64 {
        if !x.is_finite() {
            self.reset();
            return 0.;
        }
        let previous = self.previous;
        self.previous = x;
        let delta = x - previous;
        if delta.abs() < 1e-5 * (1. + x.abs().max(previous.abs())) {
            self.shape.sample(0.5 * (x + previous))
        } else {
            (self.shape.integral(x) - self.shape.integral(previous)) / delta
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub shape: Shape,
    pub drive: f32,
    pub bias: f32,
    pub mix: f32,
    pub smoothing_ms: f32,
    pub latency_multiple: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            shape: Shape::Tanh,
            drive: 1.,
            bias: 0.,
            mix: 1.,
            smoothing_ms: 5.,
            latency_multiple: 1,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), Error> {
        targets(self.drive, self.bias, self.mix)?;
        if self.schema != 1
            || ![1, 2, 4, 8].contains(&self.latency_multiple)
            || !self.smoothing_ms.is_finite()
            || !(0.0..=500.).contains(&self.smoothing_ms)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
fn targets(d: f32, b: f32, m: f32) -> Result<(), Error> {
    if !d.is_finite()
        || !(0.0..=32.).contains(&d)
        || !b.is_finite()
        || !(-2.0..=2.).contains(&b)
        || !m.is_finite()
        || !(0.0..=1.).contains(&m)
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
pub struct Waveshaper {
    config: Config,
    spec: Option<ProcessSpec>,
    states: Vec<Adaa1>,
    last_wet: Vec<f64>,
    dry: Vec<f32>,
    delay: Vec<f32>,
    cursor: usize,
    current: [f64; 3],
    coefficient: f64,
    last_error: Option<Error>,
}
impl Waveshaper {
    pub fn new(config: Config) -> Result<Self, Error> {
        config.validate()?;
        Ok(Self {
            config,
            spec: None,
            states: vec![],
            last_wet: vec![],
            dry: vec![],
            delay: vec![],
            cursor: 0,
            current: [config.drive as f64, config.bias as f64, config.mix as f64],
            coefficient: 1.,
            last_error: None,
        })
    }
    pub fn set_targets(&mut self, drive: f32, bias: f32, mix: f32) -> Result<(), Error> {
        targets(drive, bias, mix)?;
        self.config.drive = drive;
        self.config.bias = bias;
        self.config.mix = mix;
        Ok(())
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn last_error(&self) -> Option<Error> {
        self.last_error
    }
    pub fn process_checked(&mut self, b: &mut AudioBuffer<'_>) -> Result<(), Error> {
        let spec = self.spec.ok_or(Error::NotPrepared)?;
        if b.channels() != spec.channels
            || b.frames() > spec.max_block_frames
            || b.samples().iter().any(|v| !v.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        for frame in b.samples_mut().chunks_exact_mut(spec.channels) {
            for (v, target) in
                self.current
                    .iter_mut()
                    .zip([self.config.drive, self.config.bias, self.config.mix])
            {
                *v += self.coefficient * (target as f64 - *v);
            }
            let [drive, bias, mix] = self.current;
            let offset = self.config.shape.sample(bias);
            for (c, x) in frame.iter_mut().enumerate() {
                let input = *x;
                let aa = self.states[c].tick(input as f64 * drive + bias);
                let wet = 0.5 * (aa + self.last_wet[c]) - offset;
                self.last_wet[c] = aa;
                *x = ((1. - mix) * self.dry[c] as f64 + mix * wet) as f32;
                self.dry[c] = input;
                if !self.delay.is_empty() {
                    std::mem::swap(x, &mut self.delay[self.cursor]);
                    self.cursor = (self.cursor + 1) % self.delay.len();
                }
            }
        }
        Ok(())
    }
}
impl AudioProcessor for Waveshaper {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.config.validate()?;
        self.spec = Some(s);
        self.states = (0..s.channels)
            .map(|_| Adaa1::new(self.config.shape))
            .collect();
        self.last_wet = vec![0.; s.channels];
        self.dry = vec![0.; s.channels];
        self.delay = vec![0.; (self.config.latency_multiple - 1) * s.channels];
        self.coefficient = if self.config.smoothing_ms == 0. {
            1.
        } else {
            -(-1. / (s.sample_rate * self.config.smoothing_ms as f64 * 0.001)).exp_m1()
        };
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        for s in &mut self.states {
            s.reset();
        }
        self.last_wet.fill(0.);
        self.dry.fill(0.);
        self.delay.fill(0.);
        self.cursor = 0;
        self.current = [
            self.config.drive as f64,
            self.config.bias as f64,
            self.config.mix as f64,
        ];
        self.last_error = None;
    }
    fn latency_frames(&self) -> usize {
        self.config.latency_multiple
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        self.last_error = self.process_checked(b).err();
        if self.last_error.is_some() {
            b.clear();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_inputs_and_analytic_integral_are_stable() {
        for shape in [Shape::HardClip, Shape::Tanh, Shape::Cubic] {
            let mut a = Adaa1::new(shape);
            for _ in 0..100 {
                assert!(a.tick(0.3).is_finite());
            }
            assert!((a.tick(0.3) - shape.sample(0.3)).abs() < 1e-12);
            for i in 0..100 {
                let x = -2. + i as f64 * 0.04;
                let h = 1e-6;
                assert!(
                    ((shape.integral(x + h) - shape.integral(x - h)) / (2. * h) - shape.sample(x))
                        .abs()
                        < 2e-6
                );
            }
        }
    }
    #[test]
    fn integer_latency_partition_zero_alloc_and_reset() {
        let cfg = Config {
            smoothing_ms: 0.,
            ..Default::default()
        };
        let spec = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 2,
        };
        let render = |block: usize| {
            let mut p = Waveshaper::new(cfg).unwrap();
            p.prepare(spec).unwrap();
            let mut data = vec![0.; 2048];
            data[0] = 1e-6;
            for b in data.chunks_mut(block * 2) {
                let a = allocation_counter::measure(|| {
                    p.process_checked(&mut AudioBuffer::new(b, 2).unwrap())
                        .unwrap()
                });
                assert_eq!(a.count_total, 0);
            }
            let peak = data
                .iter()
                .step_by(2)
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(peak, p.latency_frames());
            assert!(data.iter().skip(1).step_by(2).all(|x| *x == 0.));
            p.reset();
            data
        };
        assert_eq!(render(128), render(1));
    }
    #[test]
    fn invalid_controls_are_transactional() {
        let mut p = Waveshaper::new(Config::default()).unwrap();
        assert!(p.set_targets(f32::NAN, 0., 1.).is_err());
        assert_eq!(p.config().drive, 1.);
        assert!(Waveshaper::new(Config {
            latency_multiple: 3,
            ..Default::default()
        })
        .is_err());
    }
}
