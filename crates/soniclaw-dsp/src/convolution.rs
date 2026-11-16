//! Uniform partitioned FFT convolution. The mono impulse is applied independently
//! to every channel. IR rate must match processing rate. One partition latency.
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use soniclaw_core::*;
use std::sync::Arc;
struct Channel {
    input: Vec<f32>,
    output: Vec<f32>,
    overlap: Vec<f32>,
    history: Vec<Vec<Complex32>>,
    spectrum: Vec<Complex32>,
    sum: Vec<Complex32>,
    cursor: usize,
}
pub struct Convolution {
    impulse: Vec<f32>,
    partition: usize,
    kernels: Vec<Vec<Complex32>>,
    channels: Vec<Channel>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex32>,
    position: usize,
    impulse_rate: f64,
}
impl Convolution {
    /// Max IR length 1,048,576 samples; partition must be power of two 32..4096.
    /// Allocates plans here and storage in prepare, never in process.
    pub fn new(impulse: Vec<f32>, impulse_rate: f64, partition: usize) -> Result<Self, Error> {
        if impulse.is_empty()
            || impulse.len() > 1_048_576
            || impulse.iter().any(|x| !x.is_finite())
            || !partition.is_power_of_two()
            || !(32..=4096).contains(&partition)
            || !impulse_rate.is_finite()
            || !(1000.0..=1_536_000.0).contains(&impulse_rate)
        {
            return Err(Error::InvalidParameter);
        }
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(partition * 2);
        let inverse = planner.plan_fft_inverse(partition * 2);
        Ok(Self {
            impulse,
            partition,
            kernels: Vec::new(),
            channels: Vec::new(),
            forward,
            inverse,
            scratch: Vec::new(),
            position: 0,
            impulse_rate,
        })
    }
    pub fn impulse_frames(&self) -> usize {
        self.impulse.len()
    }
}
impl AudioProcessor for Convolution {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if (s.sample_rate - self.impulse_rate).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        let n = self.partition * 2;
        let partitions = self.impulse.len().div_ceil(self.partition);
        // Bound frequency-domain storage before attempting allocation.
        let bytes = (partitions as u64) * (n as u64) * 8 * (s.channels as u64 + 1);
        if bytes > 512 * 1024 * 1024 {
            return Err(Error::InvalidSpec);
        }
        self.scratch = vec![
            Complex32::default();
            self.forward
                .get_inplace_scratch_len()
                .max(self.inverse.get_inplace_scratch_len())
        ];
        self.kernels = self
            .impulse
            .chunks(self.partition)
            .map(|p| {
                let mut v = vec![Complex32::default(); n];
                for (x, y) in v.iter_mut().zip(p) {
                    x.re = *y;
                }
                self.forward.process_with_scratch(&mut v, &mut self.scratch);
                v
            })
            .collect();
        self.channels = (0..s.channels)
            .map(|_| Channel {
                input: vec![0.0; self.partition],
                output: vec![0.0; self.partition],
                overlap: vec![0.0; self.partition],
                history: vec![vec![Complex32::default(); n]; partitions],
                spectrum: vec![Complex32::default(); n],
                sum: vec![Complex32::default(); n],
                cursor: 0,
            })
            .collect();
        self.position = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let count = b.channels();
        let p = self.partition;
        let k = self.kernels.len();
        for frame in b.samples_mut().chunks_exact_mut(count) {
            for (x, ch) in frame.iter_mut().zip(&mut self.channels) {
                ch.input[self.position] = *x;
                *x = ch.output[self.position];
            }
            self.position += 1;
            if self.position == p {
                for ch in &mut self.channels {
                    ch.spectrum.fill(Complex32::default());
                    for (x, y) in ch.spectrum.iter_mut().zip(&ch.input) {
                        x.re = *y;
                    }
                    self.forward
                        .process_with_scratch(&mut ch.spectrum, &mut self.scratch);
                    ch.history[ch.cursor].copy_from_slice(&ch.spectrum);
                    ch.sum.fill(Complex32::default());
                    for j in 0..k {
                        let history = &ch.history[(ch.cursor + k - j) % k];
                        for ((sum, x), h) in ch.sum.iter_mut().zip(history).zip(&self.kernels[j]) {
                            *sum += *x * *h;
                        }
                    }
                    self.inverse
                        .process_with_scratch(&mut ch.sum, &mut self.scratch);
                    let scale = 1.0 / (2 * p) as f32;
                    for i in 0..p {
                        ch.output[i] = ch.sum[i].re * scale + ch.overlap[i];
                        ch.overlap[i] = ch.sum[i + p].re * scale;
                    }
                    ch.cursor = (ch.cursor + 1) % k;
                }
                self.position = 0;
            }
        }
    }
    fn reset(&mut self) {
        self.position = 0;
        for ch in &mut self.channels {
            ch.input.fill(0.0);
            ch.output.fill(0.0);
            ch.overlap.fill(0.0);
            ch.spectrum.fill(Complex32::default());
            ch.sum.fill(Complex32::default());
            for h in &mut ch.history {
                h.fill(Complex32::default());
            }
            ch.cursor = 0;
        }
    }
    fn latency_frames(&self) -> usize {
        self.partition
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_matches_direct_convolution_across_ir_partitions_and_variable_blocks() {
        let ir: Vec<_> = (0..149)
            .map(|i| ((i * 7) as f32).sin() / (i + 1) as f32)
            .collect();
        let input: Vec<_> = (0..400).map(|i| (i as f32 * 0.17).sin()).collect();
        let mut c = Convolution::new(ir.clone(), 48000.0, 32).unwrap();
        c.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 37,
            channels: 1,
        })
        .unwrap();
        let mut actual = input.clone();
        actual.resize(600, 0.0);
        for b in actual.chunks_mut(37) {
            c.process(
                &mut AudioBuffer::new(b, 1).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
        }
        for (i, y) in actual.iter().enumerate() {
            let expected = if i < 32 {
                0.0
            } else {
                let t = i - 32;
                ir.iter()
                    .enumerate()
                    .filter_map(|(j, h)| {
                        t.checked_sub(j).and_then(|n| input.get(n)).map(|x| *x * *h)
                    })
                    .sum::<f32>()
            };
            assert!((*y - expected).abs() < 2e-5, "frame {i}: {y} != {expected}");
        }
        c.reset();
        let mut zero = [0.0; 37];
        c.process(
            &mut AudioBuffer::new(&mut zero, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert!(zero.iter().all(|x| x.abs() < 1e-7));
    }
}
