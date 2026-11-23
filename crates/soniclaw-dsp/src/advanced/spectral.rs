//! Reusable complex FFT, periodic windows and causal normalized overlap-add STFT.
#[path = "real_fft.rs"]
mod real_fft;
use super::*;
pub use real_fft::*;
pub use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;
#[derive(Clone, Copy, Debug)]
pub enum WindowKind {
    Hann,
    Hamming,
    Blackman,
}
pub fn window(kind: WindowKind, size: usize) -> Result<Vec<f32>, Error> {
    if !(16..=32768).contains(&size) {
        return Err(Error::InvalidSpec);
    }
    Ok((0..size)
        .map(|i| {
            let p = std::f32::consts::TAU * i as f32 / size as f32;
            match kind {
                WindowKind::Hann => 0.5 - 0.5 * p.cos(),
                WindowKind::Hamming => 0.54 - 0.46 * p.cos(),
                WindowKind::Blackman => 0.42 - 0.5 * p.cos() + 0.08 * (2.0 * p).cos(),
            }
        })
        .collect())
}
pub struct FftProcessor {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    size: usize,
}
impl FftProcessor {
    pub fn new(size: usize) -> Result<Self, Error> {
        if !size.is_power_of_two() || !(16..=32768).contains(&size) {
            return Err(Error::InvalidSpec);
        }
        let mut p = FftPlanner::new();
        let f = p.plan_fft_forward(size);
        let i = p.plan_fft_inverse(size);
        let scratch = vec![
            Complex32::default();
            f.get_inplace_scratch_len().max(i.get_inplace_scratch_len())
        ];
        Ok(Self {
            forward: f,
            inverse: i,
            scratch,
            size,
        })
    }
    pub fn forward(&mut self, data: &mut [Complex32]) -> Result<(), Error> {
        if data.len() != self.size {
            return Err(Error::InvalidBuffer);
        }
        self.forward.process_with_scratch(data, &mut self.scratch);
        Ok(())
    }
    pub fn inverse(&mut self, data: &mut [Complex32]) -> Result<(), Error> {
        if data.len() != self.size {
            return Err(Error::InvalidBuffer);
        }
        self.inverse.process_with_scratch(data, &mut self.scratch);
        for x in data {
            *x /= self.size as f32;
        }
        Ok(())
    }
}
pub struct Stft {
    fft: FftProcessor,
    input: Vec<f32>,
    output: Vec<f32>,
    weight: Vec<f32>,
    window: Vec<f32>,
    spectrum: Vec<Complex32>,
    input_cursor: usize,
    output_cursor: usize,
    count: usize,
    hop: usize,
    seen: usize,
}
impl Stft {
    pub fn new(size: usize, hop: usize) -> Result<Self, Error> {
        if hop == 0 || hop > size / 2 || !size.is_multiple_of(hop) {
            return Err(Error::InvalidSpec);
        }
        let fft = FftProcessor::new(size)?;
        Ok(Self {
            fft,
            input: vec![0.0; size],
            output: vec![0.0; size * 2],
            weight: vec![0.0; size * 2],
            window: window(WindowKind::Hann, size)?
                .into_iter()
                .map(|x| x.max(0.0).sqrt())
                .collect(),
            spectrum: vec![Complex32::default(); size],
            input_cursor: 0,
            output_cursor: 0,
            count: 0,
            hop,
            seen: 0,
        })
    }
    pub fn latency_frames(&self) -> usize {
        self.input.len()
    }
    /// The callback must preserve conjugate symmetry for real output and obey the
    /// real-time contract. FFT plans, windows, rings and scratch are already owned.
    pub fn tick(&mut self, x: f32, mut transform: impl FnMut(&mut [Complex32])) -> f32 {
        let size = self.input.len();
        let idx = self.output_cursor;
        let out = if self.seen >= size && self.weight[idx] > 1e-12 {
            self.output[idx] / self.weight[idx]
        } else {
            0.0
        };
        self.output[idx] = 0.0;
        self.weight[idx] = 0.0;
        if self.analyze(x) {
            transform(&mut self.spectrum);
            self.fft
                .inverse(&mut self.spectrum)
                .expect("prepared FFT size");
            for i in 0..size {
                let dest = (idx + 1 + i) % self.output.len();
                self.output[dest] += self.spectrum[i].re * self.window[i];
                self.weight[dest] += self.window[i] * self.window[i];
            }
        }
        self.output_cursor = (idx + 1) % self.output.len();
        out
    }
    /// A dedicated analysis stream shares the same window and hop as synthesis.
    /// Do not interleave analysis-only and synthesis calls on one instance.
    pub(crate) fn tick_analysis(&mut self, x: f32, mut inspect: impl FnMut(&[Complex32])) {
        if self.analyze(x) {
            inspect(&self.spectrum);
        }
    }

    fn analyze(&mut self, x: f32) -> bool {
        let size = self.input.len();
        self.seen = (self.seen + 1).min(size);
        self.input[self.input_cursor] = x;
        self.input_cursor = (self.input_cursor + 1) % size;
        self.count += 1;
        if self.count != self.hop {
            return false;
        }
        self.count = 0;
        for i in 0..size {
            self.spectrum[i] = Complex32::new(
                self.input[(self.input_cursor + i) % size] * self.window[i],
                0.0,
            );
        }
        self.fft
            .forward(&mut self.spectrum)
            .expect("prepared FFT size");
        true
    }

    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.weight.fill(0.0);
        self.spectrum.fill(Complex32::default());
        self.input_cursor = 0;
        self.output_cursor = 0;
        self.count = 0;
        self.seen = 0;
    }
}
pub struct SpectrumAnalyzer {
    fft: FftProcessor,
    window: Vec<f32>,
    data: Vec<Complex32>,
    magnitudes: Vec<f32>,
    gain: f32,
}
impl SpectrumAnalyzer {
    pub fn new(size: usize, kind: WindowKind) -> Result<Self, Error> {
        let fft = FftProcessor::new(size)?;
        let w = window(kind, size)?;
        let gain = w.iter().sum::<f32>();
        Ok(Self {
            fft,
            window: w,
            data: vec![Complex32::default(); size],
            magnitudes: vec![0.0; size / 2 + 1],
            gain,
        })
    }
    pub fn analyze(&mut self, input: &[f32]) -> Result<&[f32], Error> {
        if input.len() != self.data.len() || input.iter().any(|x| !x.is_finite()) {
            return Err(Error::InvalidBuffer);
        }
        for ((d, x), w) in self.data.iter_mut().zip(input).zip(&self.window) {
            *d = Complex32::new(*x * *w, 0.0);
        }
        self.fft.forward(&mut self.data)?;
        let last = self.magnitudes.len() - 1;
        for (i, out) in self.magnitudes.iter_mut().enumerate() {
            *out = self.data[i].norm() / self.gain * if i == 0 || i == last { 1.0 } else { 2.0 };
        }
        Ok(&self.magnitudes)
    }
}
pub struct SpectralGate {
    channels: Vec<Stft>,
    dry: Vec<f32>,
    cursor: usize,
    size: usize,
    threshold_db: f32,
    mix: f32,
}
impl Default for SpectralGate {
    fn default() -> Self {
        Self {
            channels: vec![],
            dry: vec![],
            cursor: 0,
            size: 1024,
            threshold_db: -60.0,
            mix: 1.0,
        }
    }
}
impl SpectralGate {
    pub fn set(&mut self, threshold_db: f32, mix: f32) -> Result<(), Error> {
        if !threshold_db.is_finite()
            || !(-100.0..=0.0).contains(&threshold_db)
            || !mix.is_finite()
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        self.threshold_db = threshold_db;
        self.mix = mix;
        Ok(())
    }
}
impl LiveDsp for SpectralGate {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for SpectralGate {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.channels = (0..s.channels)
            .map(|_| Stft::new(self.size, self.size / 4))
            .collect::<Result<_, _>>()?;
        self.dry = vec![0.0; self.size * s.channels];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if self.channels.len() != ch {
            b.clear();
            return;
        }
        let threshold = crate::db_to_gain(self.threshold_db) * self.size as f32 * 0.25;
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (c, x) in f.iter_mut().enumerate() {
                let wet = self.channels[c].tick(*x, |bins| {
                    for bin in bins {
                        if bin.norm() < threshold {
                            *bin = Complex32::default();
                        }
                    }
                });
                let index = self.cursor * ch + c;
                let dry = self.dry[index];
                self.dry[index] = *x;
                *x = dry * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % self.size;
        }
    }
    fn reset(&mut self) {
        for s in &mut self.channels {
            s.reset();
        }
        self.dry.fill(0.0);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.size
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_inverse_and_analyzer_calibration() {
        let mut fft = FftProcessor::new(1024).unwrap();
        let mut x: Vec<_> = (0..1024)
            .map(|i| Complex32::new((i as f32 * 0.03).sin(), 0.0))
            .collect();
        let original = x.clone();
        fft.forward(&mut x).unwrap();
        fft.inverse(&mut x).unwrap();
        assert!(x.iter().zip(original).all(|(a, b)| (*a - b).norm() < 1e-5));
        let mut a = SpectrumAnalyzer::new(1024, WindowKind::Hann).unwrap();
        let tone: Vec<_> = (0..1024)
            .map(|i| 0.5 * (std::f32::consts::TAU * 32.0 * i as f32 / 1024.0).sin())
            .collect();
        let bins = a.analyze(&tone).unwrap();
        assert!((bins[32] - 0.5).abs() < 1e-5);
    }
    #[test]
    fn stft_identity_has_exact_declared_latency() {
        for hop in [128, 256] {
            let mut s = Stft::new(512, hop).unwrap();
            let input: Vec<_> = (0..4096).map(|i| (i as f32 * 0.071).sin() * 0.3).collect();
            let mut output = vec![];
            for &x in input.iter().chain(std::iter::repeat_n(&0.0, 1024)) {
                output.push(s.tick(x, |_| {}));
            }
            assert!(output[..512].iter().all(|x| x.abs() < 1e-6));
            assert!(input
                .iter()
                .zip(&output[512..])
                .all(|(a, b)| (*a - *b).abs() < 1e-5));
            s.reset();
            assert_eq!(s.tick(0.0, |_| {}), 0.0);
        }
    }
}
