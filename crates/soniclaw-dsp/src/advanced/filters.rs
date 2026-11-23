//! Topology-preserving SVF and four-pole ladder with an analytical linear feedback
//! solution. Ladder drive saturates the input, not an iterative nonlinear loop.
use super::*;
use std::f32::consts::{FRAC_1_SQRT_2, PI};
#[derive(Clone, Copy, Debug, Default)]
pub enum FilterMode {
    #[default]
    LowPass,
    BandPass,
    HighPass,
    Notch,
}
#[derive(Clone, Copy, Default)]
struct SvfState {
    a: f32,
    b: f32,
}
impl SvfState {
    fn tick(&mut self, x: f32, g: f32, k: f32, mode: FilterMode) -> f32 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.b;
        let band = a1 * self.a + a2 * v3;
        let low = self.b + a2 * self.a + a3 * v3;
        self.a = clean(2.0 * band - self.a);
        self.b = clean(2.0 * low - self.b);
        let high = x - k * band - low;
        match mode {
            FilterMode::LowPass => low,
            FilterMode::BandPass => band,
            FilterMode::HighPass => high,
            FilterMode::Notch => low + high,
        }
    }
}
pub struct StateVariableFilter {
    states: Vec<SvfState>,
    sr: f32,
    frequency: f32,
    q: f32,
    drive: f32,
    mode: FilterMode,
    g: f32,
}
impl Default for StateVariableFilter {
    fn default() -> Self {
        Self {
            states: vec![],
            sr: 48000.0,
            frequency: 1000.0,
            q: FRAC_1_SQRT_2,
            drive: 1.0,
            mode: FilterMode::LowPass,
            g: 0.0,
        }
    }
}
impl StateVariableFilter {
    pub fn set(
        &mut self,
        frequency: f32,
        q: f32,
        drive: f32,
        mode: FilterMode,
    ) -> Result<(), Error> {
        if !frequency.is_finite()
            || !(20.0..=20000.0).contains(&frequency)
            || !q.is_finite()
            || !(0.1..=20.0).contains(&q)
            || !drive.is_finite()
            || !(0.0..=12.0).contains(&drive)
        {
            return Err(Error::InvalidParameter);
        }
        self.frequency = frequency;
        self.q = q;
        self.drive = drive;
        self.mode = mode;
        self.g = (PI * frequency.min(self.sr * 0.45) / self.sr).tan();
        Ok(())
    }
}
impl LiveDsp for StateVariableFilter {
    fn update(&mut self, v: &[f32]) {
        let mode = match v[3].round() as u8 {
            1 => FilterMode::BandPass,
            2 => FilterMode::HighPass,
            3 => FilterMode::Notch,
            _ => FilterMode::LowPass,
        };
        let _ = self.set(v[0], v[1], v[2], mode);
    }
}
impl AudioProcessor for StateVariableFilter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.states = vec![SvfState::default(); s.channels];
        self.set(self.frequency, self.q, self.drive, self.mode)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if self.states.len() != ch {
            b.clear();
            return;
        }
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (x, z) in f.iter_mut().zip(&mut self.states) {
                let input = if self.drive == 0.0 {
                    *x
                } else {
                    (*x * self.drive).tanh() / self.drive.tanh()
                };
                *x = z.tick(input, self.g, 1.0 / self.q, self.mode);
            }
        }
    }
    fn reset(&mut self) {
        self.states.fill(SvfState::default());
    }
}
pub struct LadderFilter {
    states: Vec<[f32; 4]>,
    sr: f32,
    cutoff: f32,
    resonance: f32,
    drive: f32,
    g: f32,
}
impl Default for LadderFilter {
    fn default() -> Self {
        Self {
            states: vec![],
            sr: 48000.0,
            cutoff: 1000.0,
            resonance: 0.0,
            drive: 1.0,
            g: 0.0,
        }
    }
}
impl LadderFilter {
    pub fn set(&mut self, cutoff: f32, resonance: f32, drive: f32) -> Result<(), Error> {
        if !cutoff.is_finite()
            || !(20.0..=20000.0).contains(&cutoff)
            || !resonance.is_finite()
            || !(0.0..=0.98).contains(&resonance)
            || !drive.is_finite()
            || !(0.0..=12.0).contains(&drive)
        {
            return Err(Error::InvalidParameter);
        }
        self.cutoff = cutoff;
        self.resonance = resonance;
        self.drive = drive;
        let t = (PI * cutoff.min(self.sr * 0.45) / self.sr).tan();
        self.g = t / (1.0 + t);
        Ok(())
    }
}
impl LiveDsp for LadderFilter {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2]);
    }
}
impl AudioProcessor for LadderFilter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.states = vec![[0.0; 4]; s.channels];
        self.set(self.cutoff, self.resonance, self.drive)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if self.states.len() != ch {
            b.clear();
            return;
        }
        let g = self.g;
        let k = 4.0 * self.resonance;
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (x, z) in f.iter_mut().zip(&mut self.states) {
                let input = if self.drive == 0.0 {
                    *x
                } else {
                    (*x * self.drive).tanh() / self.drive.tanh()
                };
                let sum = (1.0 - g) * (g * g * g * z[0] + g * g * z[1] + g * z[2] + z[3]);
                let mut u = (input * (1.0 + k) - k * sum) / (1.0 + k * g.powi(4));
                for s in z {
                    let v = (u - *s) * g;
                    let y = v + *s;
                    *s = clean(y + v);
                    u = y;
                }
                *x = u;
            }
        }
    }
    fn reset(&mut self) {
        self.states.fill([0.0; 4]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svf_response_and_ladder_decay() {
        let mut s = StateVariableFilter::default();
        let sp = ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 48000,
            channels: 1,
        };
        s.set(1000.0, FRAC_1_SQRT_2, 0.0, FilterMode::LowPass)
            .unwrap();
        s.prepare(sp).unwrap();
        let mut tone: Vec<_> = (0..48000)
            .map(|i| (2.0 * PI * 1000.0 * i as f32 / 48000.0).sin())
            .collect();
        s.process(&mut AudioBuffer::new(&mut tone, 1).unwrap(), &context());
        let rms = (tone[24000..].iter().map(|x| x * x).sum::<f32>() / 24000.0).sqrt();
        assert!((rms - 0.5).abs() < 0.002);
        let mut ladder = LadderFilter::default();
        ladder.set(500.0, 0.8, 2.0).unwrap();
        ladder.prepare(sp).unwrap();
        let mut impulse = vec![0.0; 48000];
        impulse[0] = 1.0;
        ladder.process(&mut AudioBuffer::new(&mut impulse, 1).unwrap(), &context());
        assert!(impulse.iter().all(|x| x.is_finite()));
        assert!(impulse[24000..].iter().all(|x| x.abs() < 1e-5));
    }
}
