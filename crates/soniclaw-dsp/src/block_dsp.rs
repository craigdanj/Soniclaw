//! Checked audio layouts with a reusable channel mixer and continuous gain ramps.
use soniclaw_core::Error;
pub use soniclaw_kernels::ClampRange;
use soniclaw_kernels::{Backend, Kernel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Planar,
    Interleaved,
}

#[derive(Clone, Copy)]
pub struct Block<'a> {
    pub(crate) data: &'a [f32],
    pub(crate) layout: Layout,
    pub(crate) channels: usize,
    pub(crate) frames: usize,
}
fn dimensions(length: usize, channels: usize) -> Result<usize, Error> {
    if !(1..=64).contains(&channels) || !length.is_multiple_of(channels) {
        return Err(Error::InvalidBuffer);
    }
    Ok(length / channels)
}
impl<'a> Block<'a> {
    pub fn new(data: &'a [f32], channels: usize, layout: Layout) -> Result<Self, Error> {
        Ok(Self {
            data,
            layout,
            channels,
            frames: dimensions(data.len(), channels)?,
        })
    }
    pub fn frames(self) -> usize {
        self.frames
    }
    pub fn sample(self, frame: usize, channel: usize) -> Option<f32> {
        if frame >= self.frames || channel >= self.channels {
            return None;
        }
        Some(self.data[index(self.layout, self.channels, self.frames, frame, channel)])
    }
}
pub struct BlockMut<'a> {
    pub(crate) data: &'a mut [f32],
    pub(crate) layout: Layout,
    pub(crate) channels: usize,
    pub(crate) frames: usize,
}
impl<'a> BlockMut<'a> {
    pub fn new(data: &'a mut [f32], channels: usize, layout: Layout) -> Result<Self, Error> {
        let frames = dimensions(data.len(), channels)?;
        Ok(Self {
            data,
            layout,
            channels,
            frames,
        })
    }
}
fn index(l: Layout, ch: usize, n: usize, i: usize, c: usize) -> usize {
    match l {
        Layout::Planar => c * n + i,
        Layout::Interleaved => i * ch + c,
    }
}

/// Elementwise math on equally shaped contiguous planar or interleaved blocks.
#[derive(Clone, Copy, Debug)]
pub struct BlockMath(Kernel);

impl BlockMath {
    pub fn new(backend: Backend) -> Result<Self, Error> {
        Kernel::new(backend)
            .map(Self)
            .map_err(|_| Error::InvalidParameter)
    }

    pub fn best() -> Self {
        Self(Kernel::best())
    }

    pub fn backend(&self) -> Backend {
        self.0.backend()
    }

    pub fn fill(&self, output: BlockMut<'_>, value: f32) {
        output.data.fill(value);
    }

    pub fn copy(&self, input: Block<'_>, output: BlockMut<'_>) -> Result<(), Error> {
        matched(input, &output)?;
        output.data.copy_from_slice(input.data);
        Ok(())
    }

    pub fn add(&self, input: Block<'_>, output: BlockMut<'_>) -> Result<(), Error> {
        matched(input, &output)?;
        self.0
            .mix_add(output.data, input.data, 1.)
            .map_err(|_| Error::InvalidBuffer)
    }

    pub fn multiply(&self, input: Block<'_>, output: BlockMut<'_>) -> Result<(), Error> {
        matched(input, &output)?;
        self.0
            .multiply(output.data, input.data)
            .map_err(|_| Error::InvalidBuffer)
    }

    pub fn clamp(&self, output: BlockMut<'_>, range: ClampRange) {
        self.0.clamp(output.data, range);
    }

    /// Whole-block peak and RMS. Empty blocks and nonfinite input are treated as silence.
    pub fn peak_rms(&self, input: Block<'_>) -> (f32, f32) {
        let mut peak: f64 = 0.;
        let mut power = 0.;
        for &value in input.data {
            if value.is_finite() {
                let value = f64::from(value);
                peak = peak.max(value.abs());
                power += value * value;
            }
        }
        (
            peak as f32,
            (power / input.data.len().max(1) as f64).sqrt() as f32,
        )
    }
}

fn matched(input: Block<'_>, output: &BlockMut<'_>) -> Result<(), Error> {
    if input.frames != output.frames
        || input.channels != output.channels
        || input.layout != output.layout
    {
        return Err(Error::InvalidBuffer);
    }
    Ok(())
}

pub struct ChannelMatrix {
    inputs: usize,
    outputs: usize,
    gains: Vec<f32>,
    kernel: Kernel,
}
impl ChannelMatrix {
    pub fn new(inputs: usize, outputs: usize, gains: Vec<f32>) -> Result<Self, Error> {
        Self::with_backend(inputs, outputs, gains, Kernel::best().backend())
    }
    pub fn with_backend(
        inputs: usize,
        outputs: usize,
        gains: Vec<f32>,
        backend: Backend,
    ) -> Result<Self, Error> {
        if !(1..=64).contains(&inputs)
            || !(1..=64).contains(&outputs)
            || gains.len() != inputs * outputs
            || gains.iter().any(|g| !g.is_finite() || g.abs() > 16.)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            inputs,
            outputs,
            gains,
            kernel: Kernel::new(backend).map_err(|_| Error::InvalidParameter)?,
        })
    }
    pub fn backend(&self) -> Backend {
        self.kernel.backend()
    }
    pub fn mix(&self, input: Block<'_>, output: BlockMut<'_>) -> Result<(), Error> {
        if input.channels != self.inputs
            || output.channels != self.outputs
            || input.frames != output.frames
        {
            return Err(Error::InvalidBuffer);
        }
        let frames = input.frames;
        if frames == 0 {
            return Ok(());
        }
        if input.layout == Layout::Planar && output.layout == Layout::Planar {
            for (row, dst) in self
                .gains
                .chunks_exact(self.inputs)
                .zip(output.data.chunks_exact_mut(frames))
            {
                dst.fill(0.);
                for (&g, src) in row.iter().zip(input.data.chunks_exact(frames)) {
                    if g != 0. {
                        self.kernel
                            .mix_add(dst, src, g)
                            .expect("validated dimensions and gain");
                    }
                }
            }
        } else {
            for frame in 0..frames {
                for (o, row) in self.gains.chunks_exact(self.inputs).enumerate() {
                    let mut sum = 0.;
                    for (i, &g) in row.iter().enumerate() {
                        if g != 0. {
                            sum +=
                                input.data[index(input.layout, self.inputs, frames, frame, i)] * g;
                        }
                    }
                    output.data[index(output.layout, self.outputs, frames, frame, o)] = sum;
                }
            }
        }
        Ok(())
    }
}
pub struct GainRamp {
    current: f64,
    target: f64,
    step: f64,
    remaining: usize,
}
impl GainRamp {
    pub fn new(gain: f32) -> Result<Self, Error> {
        if !gain.is_finite() {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            current: f64::from(gain),
            target: f64::from(gain),
            step: 0.,
            remaining: 0,
        })
    }
    pub fn set(&mut self, target: f32, frames: usize) -> Result<(), Error> {
        if !target.is_finite() {
            return Err(Error::InvalidParameter);
        }
        self.target = f64::from(target);
        self.remaining = frames;
        if frames == 0 {
            self.current = self.target;
            self.step = 0.;
        } else {
            self.step = (self.target - self.current) / frames as f64;
        }
        Ok(())
    }
    pub fn apply(&mut self, b: BlockMut<'_>) {
        for i in 0..b.frames {
            if self.remaining > 0 {
                self.remaining -= 1;
                self.current = if self.remaining == 0 {
                    self.target
                } else {
                    self.current + self.step
                };
            }
            for c in 0..b.channels {
                b.data[index(b.layout, b.channels, b.frames, i, c)] *= self.current as f32;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_and_scalar_agree() {
        let gains = vec![0.3, -0.2, 0.85, 0.5];
        let m = ChannelMatrix::new(2, 2, gains.clone()).unwrap();
        let scalar = ChannelMatrix::with_backend(2, 2, gains, Backend::Scalar).unwrap();
        let pcm: Vec<_> = (0..128).map(|i| (i as f32 * 0.7).sin()).collect();
        let mut a = [0.; 128];
        let mut b = a;
        let mut c = a;
        let input = Block::new(&pcm, 2, Layout::Planar).unwrap();
        m.mix(input, BlockMut::new(&mut a, 2, Layout::Planar).unwrap())
            .unwrap();
        scalar
            .mix(input, BlockMut::new(&mut b, 2, Layout::Planar).unwrap())
            .unwrap();
        m.mix(
            input,
            BlockMut::new(&mut c, 2, Layout::Interleaved).unwrap(),
        )
        .unwrap();
        for i in 0..64 {
            for ch in 0..2 {
                assert!((a[ch * 64 + i] - b[ch * 64 + i]).abs() < 1e-6);
                assert!((a[ch * 64 + i] - c[2 * i + ch]).abs() < 1e-6);
            }
        }
    }
}
