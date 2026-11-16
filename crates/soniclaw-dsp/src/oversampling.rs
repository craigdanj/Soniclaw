//! Two-times oversampled soft clipping, using two 63-tap windowed-sinc
//! low-pass FIRs. Reports 31 base-rate frames of linear-phase latency.
use soniclaw_core::*;
use std::f32::consts::PI;
const TAPS: usize = 63;
#[derive(Clone)]
struct Fir {
    data: [f32; TAPS],
    cursor: usize,
}
impl Default for Fir {
    fn default() -> Self {
        Self {
            data: [0.0; TAPS],
            cursor: 0,
        }
    }
}
impl Fir {
    fn tick(&mut self, x: f32, h: &[f32; TAPS]) -> f32 {
        self.data[self.cursor] = x;
        let mut y = 0.0;
        for (i, c) in h.iter().enumerate() {
            y += *c * self.data[(self.cursor + TAPS - i) % TAPS];
        }
        self.cursor = (self.cursor + 1) % TAPS;
        y
    }
}
pub struct OversampledDrive {
    drive: f32,
    wet: f32,
    taps: [f32; TAPS],
    up: Vec<Fir>,
    down: Vec<Fir>,
    dry: Vec<[f32; 31]>,
    cursor: usize,
}
impl OversampledDrive {
    pub fn new(drive: f32, wet: f32) -> Result<Self, Error> {
        if !drive.is_finite()
            || !(0.0..=32.0).contains(&drive)
            || !wet.is_finite()
            || !(0.0..=1.0).contains(&wet)
        {
            return Err(Error::InvalidParameter);
        }
        let mut taps = [0.0; TAPS];
        for (i, t) in taps.iter_mut().enumerate() {
            let x = i as f32 - 31.0;
            let sinc = if x == 0.0 {
                0.5
            } else {
                (PI * 0.5 * x).sin() / (PI * x)
            };
            *t = sinc
                * (0.42 - 0.5 * (2.0 * PI * i as f32 / 62.0).cos()
                    + 0.08 * (4.0 * PI * i as f32 / 62.0).cos());
        }
        let sum = taps.iter().sum::<f32>();
        for t in &mut taps {
            *t /= sum;
        }
        Ok(Self {
            drive,
            wet,
            taps,
            up: Vec::new(),
            down: Vec::new(),
            dry: Vec::new(),
            cursor: 0,
        })
    }
}
impl AudioProcessor for OversampledDrive {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.up = vec![Fir::default(); s.channels];
        self.down = vec![Fir::default(); s.channels];
        self.dry = vec![[0.0; 31]; s.channels];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let channels = b.channels();
        for f in b.samples_mut().chunks_exact_mut(channels) {
            for (c, x) in f.iter_mut().enumerate() {
                let dry = self.dry[c][self.cursor];
                self.dry[c][self.cursor] = *x;
                let a = 2.0 * self.up[c].tick(*x, &self.taps);
                let clipped = if self.drive == 0.0 {
                    a
                } else {
                    (a * self.drive).tanh() / self.drive.tanh()
                };
                let wet = self.down[c].tick(clipped, &self.taps);
                let a = 2.0 * self.up[c].tick(0.0, &self.taps);
                let clipped = if self.drive == 0.0 {
                    a
                } else {
                    (a * self.drive).tanh() / self.drive.tanh()
                };
                let _ = self.down[c].tick(clipped, &self.taps);
                *x = dry * (1.0 - self.wet) + wet * self.wet;
            }
            self.cursor = (self.cursor + 1) % 31;
        }
    }
    fn reset(&mut self) {
        self.up.fill(Fir::default());
        self.down.fill(Fir::default());
        self.dry.fill([0.0; 31]);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        31
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reported_latency_matches_impulse_peak_and_dry() {
        for wet in [0.0, 1.0] {
            let mut p = OversampledDrive::new(0.0, wet).unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.0,
                max_block_frames: 128,
                channels: 1,
            })
            .unwrap();
            let mut d = [0.0; 128];
            d[0] = 1.0;
            p.process(
                &mut AudioBuffer::new(&mut d, 1).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
            let peak = d
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0;
            assert_eq!(peak, p.latency_frames());
            assert!((d.iter().sum::<f32>() - 1.0).abs() < 0.001);
        }
    }
}
