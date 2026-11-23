//! Eight-line orthogonal Householder FDN, damped frequency-dependent decay,
//! early reflections, predelay, input diffusion and fractional-delay modulation.
use super::*;
use std::f32::consts::TAU;
struct Line {
    data: Vec<f32>,
    cursor: usize,
    delay: f32,
    damped: f32,
    gain: f32,
}
struct AllPass {
    data: Vec<f32>,
    cursor: usize,
}
impl AllPass {
    fn tick(&mut self, x: f32, g: f32) -> f32 {
        let delayed = self.data[self.cursor];
        let y = delayed - g * x;
        self.data[self.cursor] = clean(x + g * y);
        self.cursor = (self.cursor + 1) % self.data.len();
        y
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.cursor = 0;
    }
}
pub struct FdnReverb {
    lines: Vec<Line>,
    diffusers: Vec<[AllPass; 2]>,
    early: Vec<f32>,
    early_frames: usize,
    cursor: usize,
    channels: usize,
    sr: f32,
    phase: f32,
    rt60: f32,
    mix: f32,
    damping: f32,
    predelay: f32,
    diffusion: f32,
    modulation: f32,
    damp_pole: f32,
}
impl Default for FdnReverb {
    fn default() -> Self {
        Self {
            lines: vec![],
            diffusers: vec![],
            early: vec![],
            early_frames: 0,
            cursor: 0,
            channels: 0,
            sr: 48000.0,
            phase: 0.0,
            rt60: 2.0,
            mix: 0.25,
            damping: 6000.0,
            predelay: 15.0,
            diffusion: 0.7,
            modulation: 0.3,
            damp_pole: 0.0,
        }
    }
}
impl FdnReverb {
    pub fn set(
        &mut self,
        rt60: f32,
        mix: f32,
        damping: f32,
        predelay: f32,
        diffusion: f32,
        modulation: f32,
    ) -> Result<(), Error> {
        let values = [rt60, mix, damping, predelay, diffusion, modulation];
        let ranges = [
            0.1..=15.0,
            0.0..=1.0,
            500.0..=16000.0,
            0.0..=200.0,
            0.0..=1.0,
            0.0..=1.0,
        ];
        if values
            .iter()
            .zip(ranges)
            .any(|(v, r)| !v.is_finite() || !r.contains(v))
        {
            return Err(Error::InvalidParameter);
        }
        if self.rt60 != rt60 {
            for line in &mut self.lines {
                line.gain = 10.0_f32.powf(-3.0 * line.delay / (self.sr * rt60));
            }
        }
        self.rt60 = rt60;
        self.mix = mix;
        self.damping = damping;
        self.predelay = predelay;
        self.diffusion = diffusion;
        self.modulation = modulation;
        self.damp_pole = (-TAU * damping.min(self.sr * 0.45) / self.sr).exp();
        Ok(())
    }
    fn early_read(&self, ms: f32, c: usize) -> f32 {
        let frames = ms * self.sr * 0.001;
        let pos = (self.cursor as f32 - frames).rem_euclid(self.early_frames as f32);
        let a = pos as usize % self.early_frames;
        let frac = pos.fract();
        self.early[a * self.channels + c] * (1.0 - frac)
            + self.early[((a + 1) % self.early_frames) * self.channels + c] * frac
    }
}
impl LiveDsp for FdnReverb {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2], v[3], v[4], v[5]);
    }
}
impl AudioProcessor for FdnReverb {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.lines = [
            0.0297, 0.0371, 0.0411, 0.0437, 0.0533, 0.0617, 0.0719, 0.0833,
        ]
        .into_iter()
        .map(|seconds| {
            let delay = (seconds * self.sr).round().max(8.0);
            Line {
                data: vec![0.0; delay as usize + 32],
                cursor: 0,
                delay,
                damped: 0.0,
                gain: 10.0_f32.powf(-3.0 * delay / (self.sr * self.rt60)),
            }
        })
        .collect();
        self.diffusers = (0..s.channels)
            .map(|c| {
                std::array::from_fn(|i| AllPass {
                    data: vec![
                        0.0;
                        ((0.0017 + i as f32 * 0.0031 + c as f32 * 0.00007) * self.sr)
                            .round()
                            .max(1.0) as usize
                    ],
                    cursor: 0,
                })
            })
            .collect();
        self.early_frames = (self.sr * 0.25).ceil() as usize + 2;
        self.early = vec![0.0; self.early_frames * s.channels];
        self.set(
            self.rt60,
            self.mix,
            self.damping,
            self.predelay,
            self.diffusion,
            self.modulation,
        )?;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels || self.lines.len() != 8 {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            for (c, x) in frame.iter().enumerate() {
                self.early[self.cursor * self.channels + c] = *x;
            }
            let mut excitation = [0.0; 64];
            let mut reflections = [0.0; 64];
            for c in 0..self.channels {
                let mut x = self.early_read(self.predelay, c);
                reflections[c] = self.early_read(self.predelay + 9.0, c) * 0.3
                    + self.early_read(self.predelay + 23.0, c) * 0.2
                    + self.early_read(self.predelay + 37.0, c) * 0.1;
                for diffuser in &mut self.diffusers[c] {
                    let wet = diffuser.tick(x, self.diffusion * 0.65);
                    x = x * (1.0 - self.diffusion) + wet * self.diffusion;
                }
                excitation[c] = x;
            }
            let left = excitation[0];
            let right = if self.channels >= 2 {
                excitation[1]
            } else {
                left
            };
            let mut values = [0.0; 8];
            for (i, line) in self.lines.iter_mut().enumerate() {
                let modulation = 8.0
                    * self.modulation
                    * (self.phase * (1.0 + i as f32 * 0.037) + i as f32 * 0.71).sin();
                let pos = (line.cursor as f32 - line.delay - modulation)
                    .rem_euclid(line.data.len() as f32);
                let a = pos as usize % line.data.len();
                let frac = pos.fract();
                let y = line.data[a] * (1.0 - frac) + line.data[(a + 1) % line.data.len()] * frac;
                line.damped = clean((1.0 - self.damp_pole) * y + self.damp_pole * line.damped);
                values[i] = line.damped;
            }
            let sum = values.iter().sum::<f32>();
            let mut late = [0.0; 2];
            for (i, line) in self.lines.iter_mut().enumerate() {
                let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                let input = (left + right * sign) * 0.25;
                line.data[line.cursor] = clean(input + (values[i] - sum * 0.25) * line.gain);
                line.cursor = (line.cursor + 1) % line.data.len();
                late[0] += values[i] * 0.35355338;
                late[1] += values[i] * sign * 0.35355338;
            }
            for (c, x) in frame.iter_mut().enumerate() {
                let wet = late[c % 2] + reflections[c];
                *x = *x * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % self.early_frames;
            self.phase = (self.phase + TAU * 0.23 / self.sr).rem_euclid(TAU);
        }
    }
    fn reset(&mut self) {
        for line in &mut self.lines {
            line.data.fill(0.0);
            line.cursor = 0;
            line.damped = 0.0;
        }
        for channel in &mut self.diffusers {
            for a in channel {
                a.reset();
            }
        }
        self.early.fill(0.0);
        self.cursor = 0;
        self.phase = 0.0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverb_has_predelay_decay_and_reset() {
        let mut r = FdnReverb::default();
        r.set(0.4, 1.0, 8000.0, 30.0, 0.5, 0.0).unwrap();
        r.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 96000,
            channels: 2,
        })
        .unwrap();
        let mut d = vec![0.0; 192000];
        d[0] = 1.0;
        d[1] = 1.0;
        r.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &context());
        assert!(d[..2880].iter().all(|x| *x == 0.0));
        assert!(d[2880..48000].iter().any(|x| x.abs() > 1e-4));
        let early = d[5000..25000].iter().map(|x| x * x).sum::<f32>();
        let late = d[150000..190000].iter().map(|x| x * x).sum::<f32>();
        assert!(late < early * 0.001);
        assert!(d.iter().all(|x| x.is_finite()));
        r.reset();
        d.fill(0.0);
        r.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &context());
        assert!(d.iter().all(|x| *x == 0.0));
    }
}
