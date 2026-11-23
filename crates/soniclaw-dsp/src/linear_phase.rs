//! Symmetric crossover bands that sum to a pure delay.
use crate::filter_design::{self, Response};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

pub struct Crossover {
    low: Vec<f64>,
    high_edge: Vec<f64>,
    history: Vec<f64>,
    cursor: usize,
    channels: usize,
    rate: f64,
}
impl Crossover {
    pub fn new(
        rate: f64,
        channels: usize,
        edges: [f64; 2],
        transition: f64,
        attenuation: f64,
    ) -> Result<Self, Error> {
        if !(1..=64).contains(&channels) || edges[1] - edges[0] < transition {
            return Err(Error::InvalidParameter);
        }
        let low =
            filter_design::kaiser(rate, edges[0], transition, attenuation, Response::LowPass)?;
        let high_edge =
            filter_design::kaiser(rate, edges[1], transition, attenuation, Response::LowPass)?;
        let len = low.len();
        Ok(Self {
            low,
            high_edge,
            history: vec![0.; len * channels],
            cursor: 0,
            channels,
            rate,
        })
    }
    pub fn latency_frames(&self) -> usize {
        self.low.len() / 2
    }
    pub fn reset(&mut self) {
        self.history.fill(0.);
        self.cursor = 0;
    }
    fn one(&mut self, c: usize, input: f32) -> [f32; 3] {
        let n = self.low.len();
        let d = n / 2;
        let history = &mut self.history[c * n..][..n];
        history[self.cursor] = if input.is_finite() {
            f64::from(input)
        } else {
            0.
        };
        let (before, after) = history.split_at(self.cursor + 1);
        let (a, b) = before
            .iter()
            .rev()
            .chain(after.iter().rev())
            .zip(self.low.iter().zip(&self.high_edge))
            .fold((0., 0.), |(a, b), (x, (l, h))| (a + l * x, b + h * x));
        let delayed = history[(self.cursor + n - d) % n];
        [a as f32, (b - a) as f32, (delayed - b) as f32]
    }
    /// Outputs are separate interleaved buffers in low, middle, high order.
    pub fn split(&mut self, input: &[f32], outputs: [&mut [f32]; 3]) -> Result<(), Error> {
        let [low, mid, high] = outputs;
        if !input.len().is_multiple_of(self.channels)
            || [low.len(), mid.len(), high.len()]
                .into_iter()
                .any(|n| n != input.len())
        {
            return Err(Error::InvalidBuffer);
        }
        for (i, &sample) in input.iter().enumerate() {
            let c = i % self.channels;
            [low[i], mid[i], high[i]] = self.one(c, sample);
            if c + 1 == self.channels {
                self.advance();
            }
        }
        Ok(())
    }
    fn advance(&mut self) {
        self.cursor = (self.cursor + 1) % self.low.len();
    }
}

pub struct Equalizer {
    crossover: Crossover,
    gains: [f32; 3],
    spec: Option<ProcessSpec>,
}
impl Equalizer {
    pub fn new(crossover: Crossover, gains: [f32; 3]) -> Result<Self, Error> {
        let mut eq = Self {
            crossover,
            gains: [1.; 3],
            spec: None,
        };
        eq.set_gains(gains)?;
        Ok(eq)
    }
    pub fn set_gains(&mut self, gains: [f32; 3]) -> Result<(), Error> {
        if gains.iter().any(|g| !(0. ..=8.).contains(g)) {
            return Err(Error::InvalidParameter);
        }
        self.gains = gains;
        Ok(())
    }
}
impl AudioProcessor for Equalizer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != self.crossover.channels
            || (s.sample_rate - self.crossover.rate).abs() > 0.001
        {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        self.spec = Some(s);
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
        for f in b.samples_mut().chunks_exact_mut(s.channels) {
            for (c, y) in f.iter_mut().enumerate() {
                *y = self
                    .crossover
                    .one(c, *y)
                    .iter()
                    .zip(self.gains)
                    .map(|(a, g)| a * g)
                    .sum();
            }
            self.crossover.advance();
        }
    }
    fn reset(&mut self) {
        self.crossover.reset();
    }
    fn latency_frames(&self) -> usize {
        self.crossover.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_bands_recombine_with_reported_delay() {
        let mut c = Crossover::new(48000., 2, [2000., 6000.], 1000., 65.).unwrap();
        let mut input = vec![0.; 2000];
        input[0] = 1.;
        input[3] = -0.5;
        let mut a = vec![0.; input.len()];
        let mut b = a.clone();
        let mut d = a.clone();
        c.split(&input, [&mut a, &mut b, &mut d]).unwrap();
        let delay = c.latency_frames() * 2;
        for (i, ((a, b), d)) in a.iter().zip(b).zip(d).enumerate() {
            let expected = if i >= delay { input[i - delay] } else { 0. };
            assert!((a + b + d - expected).abs() < 1e-6);
        }
    }
}
