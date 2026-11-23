//! Profile-based spectral attenuation, harmonic notches and isolated click repair.
use crate::advanced::spectral::Stft;
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileMode {
    Learn,
    Frozen,
    Adaptive,
}

struct NoiseChannel {
    stft: Stft,
    power: Vec<f32>,
    gains: Vec<f32>,
    learned: bool,
}
pub struct Denoiser {
    size: usize,
    strength: f32,
    floor: f32,
    mode: ProfileMode,
    spec: Option<ProcessSpec>,
    channels: Vec<NoiseChannel>,
}
impl Denoiser {
    pub fn new(size: usize, strength: f32, floor: f32) -> Result<Self, Error> {
        if !size.is_power_of_two()
            || !(256..=4096).contains(&size)
            || !(0. ..=4.).contains(&strength)
            || !(0.01..=1.).contains(&floor)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            size,
            strength,
            floor,
            mode: ProfileMode::Learn,
            spec: None,
            channels: Vec::new(),
        })
    }
    pub fn set_mode(&mut self, mode: ProfileMode) {
        if mode == ProfileMode::Learn && self.mode != mode {
            for c in &mut self.channels {
                c.learned = false;
                c.power.fill(0.);
                c.gains.fill(1.);
            }
        }
        self.mode = mode;
    }
}
impl AudioProcessor for Denoiser {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels > 8 {
            return Err(Error::InvalidSpec);
        }
        self.channels = (0..spec.channels)
            .map(|_| {
                Ok(NoiseChannel {
                    stft: Stft::new(self.size, self.size / 4)?,
                    power: vec![0.; self.size / 2 + 1],
                    gains: vec![1.; self.size / 2 + 1],
                    learned: false,
                })
            })
            .collect::<Result<_, Error>>()?;
        self.spec = Some(spec);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            for (sample, c) in frame.iter_mut().zip(&mut self.channels) {
                let x = if sample.is_finite() {
                    sample.clamp(-32., 32.)
                } else {
                    0.
                };
                let NoiseChannel {
                    stft,
                    power,
                    gains,
                    learned,
                } = c;
                *sample = stft.tick(x, |bins| {
                    let n = bins.len();
                    for (k, (noise, gain)) in power.iter_mut().zip(gains.iter_mut()).enumerate() {
                        let p = bins[k].norm_sqr();
                        if self.mode == ProfileMode::Learn
                            || !(*learned) && self.mode == ProfileMode::Adaptive
                        {
                            *noise = if *learned {
                                *noise + 0.15 * (p - *noise)
                            } else {
                                p
                            };
                            *gain = 1.;
                            continue;
                        }
                        if !*learned {
                            continue;
                        }
                        if self.mode == ProfileMode::Adaptive && p < *noise * 2.5 {
                            *noise += 0.01 * (p - *noise);
                        }
                        let target = (1. - self.strength * *noise / (p + 1e-12))
                            .max(self.floor * self.floor)
                            .sqrt();
                        *gain += 0.25 * (target - *gain);
                        bins[k] *= *gain;
                        if k > 0 && k < n / 2 {
                            bins[n - k] *= *gain;
                        }
                    }
                    if self.mode != ProfileMode::Frozen {
                        *learned = true;
                    }
                });
            }
        }
    }
    fn reset(&mut self) {
        for c in &mut self.channels {
            c.stft.reset();
            c.gains.fill(1.);
        }
    }
    fn latency_frames(&self) -> usize {
        self.size
    }
}

pub struct Dehum {
    coefficients: Vec<([f64; 3], [f64; 2])>,
    states: Vec<[f64; 2]>,
    channels: usize,
}
impl Dehum {
    pub fn new(
        rate: f64,
        channels: usize,
        fundamental: f64,
        harmonics: usize,
        q: f64,
    ) -> Result<Self, Error> {
        if !(8000. ..=384000.).contains(&rate)
            || !(1..=64).contains(&channels)
            || !(40. ..=70.).contains(&fundamental)
            || !(1..=24).contains(&harmonics)
            || !(5. ..=100.).contains(&q)
            || fundamental * harmonics as f64 >= rate * 0.45
        {
            return Err(Error::InvalidParameter);
        }
        let coefficients = (1..=harmonics)
            .map(|h| {
                let w = std::f64::consts::TAU * fundamental * h as f64 / rate;
                let a = w.sin() / (2. * q);
                let norm = 1. / (1. + a);
                let b = -2. * w.cos() * norm;
                ([norm, b, norm], [b, (1. - a) * norm])
            })
            .collect();
        Ok(Self {
            coefficients,
            states: vec![[0.; 2]; channels * harmonics],
            channels,
        })
    }
    pub fn process(&mut self, data: &mut [f32]) -> Result<(), Error> {
        if !data.len().is_multiple_of(self.channels) || data.iter().any(|x| !x.is_finite()) {
            return Err(Error::InvalidBuffer);
        }
        for frame in data.chunks_exact_mut(self.channels) {
            for (sample, history) in frame
                .iter_mut()
                .zip(self.states.chunks_exact_mut(self.coefficients.len()))
            {
                let mut x = f64::from(*sample);
                for ((b, a), z) in self.coefficients.iter().zip(history) {
                    let y = b[0] * x + z[0];
                    z[0] = b[1] * x - a[0] * y + z[1];
                    z[1] = b[2] * x - a[1] * y;
                    x = y;
                }
                *sample = x as f32;
            }
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        self.states.fill([0.; 2]);
    }
}

/// Offline isolated outlier repair. Returns the number of replaced samples.
pub fn declick(
    input: &[f32],
    output: &mut [f32],
    channels: usize,
    threshold: f32,
) -> Result<usize, Error> {
    if !(1..=64).contains(&channels)
        || !(0.0001..=2.).contains(&threshold)
        || input.len() != output.len()
        || !input.len().is_multiple_of(channels)
        || input.iter().any(|x| !x.is_finite())
    {
        return Err(Error::InvalidBuffer);
    }
    output.copy_from_slice(input);
    let mut count = 0;
    let distance = channels;
    for i in distance..input.len().saturating_sub(distance) {
        let a = input[i - distance];
        let b = input[i + distance];
        let center = 0.5 * a + 0.5 * b;
        if (a - b).abs() < threshold * 0.25 && (input[i] - center).abs() > threshold {
            output[i] = center;
            count += 1;
        }
    }
    Ok(count)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn click_repair_preserves_channels() {
        let a = [0., 0., 1., 0.02, 0., 0.04, 0., 0.06];
        let mut b = a;
        assert_eq!(declick(&a, &mut b, 2, 0.25).unwrap(), 1);
        assert_eq!(b, [0., 0., 0., 0.02, 0., 0.04, 0., 0.06]);
    }
    #[test]
    fn notch_at_mains_frequency() {
        let mut d = Dehum::new(48000., 1, 60., 3, 5.).unwrap();
        let mut x: Vec<_> = (0..48000)
            .map(|i| (std::f64::consts::TAU * 60. * i as f64 / 48000.).sin() as f32)
            .collect();
        d.process(&mut x).unwrap();
        assert!(x[40000..].iter().all(|x| x.abs() < 1e-5));
    }
}
