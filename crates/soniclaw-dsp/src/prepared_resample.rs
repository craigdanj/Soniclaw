//! Prepared normalized polyphase sinc kernels; immutable storage may be shared across voices.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
use soniclaw_kernels::{Backend, Kernel};
use std::f64::consts::PI;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub phases: usize,
    pub base_radius: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            phases: 512,
            base_radius: 16,
        }
    }
}
struct Band {
    increment: f64,
    radius: usize,
    table: Vec<f32>,
}
pub struct PreparedSinc {
    config: Config,
    bands: [Band; 4],
    kernel: Kernel,
    bytes: usize,
}
impl PreparedSinc {
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.schema != 1
            || !config.phases.is_power_of_two()
            || !(64..=2048).contains(&config.phases)
            || !(8..=32).contains(&config.base_radius)
        {
            return Err(Error::InvalidParameter);
        }
        let bytes = (config.phases + 1) * 2 * config.base_radius * 15 * std::mem::size_of::<f32>();
        if bytes > 8 * 1024 * 1024 {
            return Err(Error::InvalidParameter);
        }
        let bands = std::array::from_fn(|index| {
            let increment = (1usize << index) as f64;
            let radius = config.base_radius * (1usize << index);
            let taps = radius * 2;
            let cutoff = 0.94 / increment;
            let mut table = vec![0.; (config.phases + 1) * taps];
            for phase in 0..=config.phases {
                let fraction = phase as f64 / config.phases as f64;
                let row = &mut table[phase * taps..(phase + 1) * taps];
                let mut norm = 0.;
                for (j, w) in row.iter_mut().enumerate() {
                    let x = j as f64 - radius as f64 + 1. - fraction;
                    let u = x / radius as f64;
                    let window = 0.42 + 0.5 * (PI * u).cos() + 0.08 * (2. * PI * u).cos();
                    let sinc = if x.abs() < 1e-12 {
                        cutoff
                    } else {
                        (PI * cutoff * x).sin() / (PI * x)
                    };
                    *w = (sinc * window) as f32;
                    norm += *w as f64;
                }
                for w in row {
                    *w = (*w as f64 / norm) as f32;
                }
            }
            Band {
                increment,
                radius,
                table,
            }
        });
        Ok(Self {
            config,
            bands,
            kernel: Kernel::best(),
            bytes,
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn storage_bytes(&self) -> usize {
        self.bytes
    }
    pub fn backend(&self) -> Backend {
        self.kernel.backend()
    }
    /// Uses the smallest cutoff band covering increment (1/2/4/8). Caller supplies padding/loop wrapping.
    /// No trigonometry, allocation, CPU detection or shared-state mutation in this method.
    pub fn interpolate(
        &self,
        position: f64,
        increment: f64,
        mut read: impl FnMut(isize) -> f32,
    ) -> f32 {
        if !position.is_finite()
            || position < 0.
            || position > isize::MAX as f64 / 4.
            || !increment.is_finite()
            || increment <= 0.
            || increment > 8.
        {
            return 0.;
        }
        let band = self
            .bands
            .iter()
            .find(|b| increment <= b.increment)
            .expect("bounded");
        let base = position.floor() as isize;
        let phase = position.fract() * self.config.phases as f64;
        let index = phase.floor() as usize;
        let blend = (phase - index as f64) as f32;
        let taps = band.radius * 2;
        let a = &band.table[index * taps..(index + 1) * taps];
        let b = &band.table[(index + 1) * taps..(index + 2) * taps];
        let mut weights = [0f32; 512];
        let mut samples = [0f32; 512];
        for j in 0..taps {
            weights[j] = a[j] + (b[j] - a[j]) * blend;
            samples[j] = read(base + j as isize - band.radius as isize + 1);
        }
        self.kernel
            .dot(&weights[..taps], &samples[..taps])
            .expect("equal bounded slices") as f32
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::resample::ResampleQuality;
    #[test]
    fn coefficients_match_reference_at_band_rates_and_allocate_nothing() {
        let p = PreparedSinc::new(Default::default()).unwrap();
        for rate in [1., 2., 4., 8.] {
            for phase in [0., 0.12345, 0.5, 0.9999] {
                let read = |i: isize| (i as f64 * 0.2).sin() as f32;
                let position = 4096. + phase;
                let expected = ResampleQuality::SincFast.interpolate(position, rate, read);
                let mut actual = 0.;
                assert_eq!(
                    allocation_counter::measure(|| actual = p.interpolate(position, rate, read))
                        .count_total,
                    0
                );
                assert!(
                    (actual - expected).abs() < 2e-6,
                    "{rate} {phase} {actual} {expected}"
                );
                assert!((p.interpolate(position, rate, |_| 1.) - 1.).abs() < 2e-6);
            }
        }
        assert_eq!(p.interpolate(1., 9., |_| 1.), 0.);
        assert!(PreparedSinc::new(Config {
            phases: 63,
            ..Default::default()
        })
        .is_err());
    }
    #[test]
    fn above_nyquist_fixture_is_attenuated() {
        let p = PreparedSinc::new(Default::default()).unwrap();
        let rms = |freq: f64| {
            ((0..1024)
                .map(|i| {
                    p.interpolate(10000. + i as f64 * 4., 4., |j| {
                        (std::f64::consts::TAU * freq * j as f64).sin() as f32
                    }) as f64
                })
                .map(|x| x * x)
                .sum::<f64>()
                / 1024.)
                .sqrt()
        };
        assert!(rms(0.2) < rms(0.02) * 0.001);
    }
}
