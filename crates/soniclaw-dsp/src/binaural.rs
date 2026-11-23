//! SOFA HRTF interpolation on the control side, prepared convolution on the audio side.
use crate::stereo_convolution::{
    convolution_pair, ConvolutionController, ConvolutionMode, ReloadableConvolution, StereoImpulse,
};
use sofar::reader::{Filter, OpenOptions, Sofar};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::{fmt, io::Read, path::Path};

const MAX_SOFA_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILTER_FRAMES: usize = 8192;

#[derive(Debug)]
pub enum SofaError {
    Io(std::io::Error),
    Decode(sofar::reader::Error),
    UnsupportedDataset,
}
impl fmt::Display for SofaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "SOFA read failed: {e}"),
            Self::Decode(e) => write!(f, "SOFA decode failed: {e}"),
            Self::UnsupportedDataset => write!(f, "unsupported SOFA size, rate or receiver layout"),
        }
    }
}
impl std::error::Error for SofaError {}

#[derive(Clone, Copy, Debug)]
pub struct Direction {
    /// Degrees, positive toward the left ear.
    pub azimuth: f32,
    pub elevation: f32,
    pub distance_metres: f32,
}
impl Direction {
    fn cartesian(self) -> Result<[f32; 3], Error> {
        if !(-360. ..=360.).contains(&self.azimuth)
            || !(-90. ..=90.).contains(&self.elevation)
            || !(0.05..=25.).contains(&self.distance_metres)
        {
            return Err(Error::InvalidParameter);
        }
        let (az_sin, az_cos) = self.azimuth.to_radians().sin_cos();
        let (el_sin, el_cos) = self.elevation.to_radians().sin_cos();
        let r = self.distance_metres;
        Ok([r * el_cos * az_cos, r * el_cos * az_sin, r * el_sin])
    }
}

pub struct HrtfSet {
    sofa: Sofar,
    rate: u32,
}
impl HrtfSet {
    pub fn open(path: impl AsRef<Path>, sample_rate: u32) -> Result<Self, SofaError> {
        let file = std::fs::File::open(path).map_err(SofaError::Io)?;
        let mut data = Vec::new();
        file.take(MAX_SOFA_BYTES as u64 + 1)
            .read_to_end(&mut data)
            .map_err(SofaError::Io)?;
        Self::from_bytes(&data, sample_rate)
    }

    pub fn from_bytes(data: &[u8], sample_rate: u32) -> Result<Self, SofaError> {
        if data.len() > MAX_SOFA_BYTES || !(8_000..=192_000).contains(&sample_rate) {
            return Err(SofaError::UnsupportedDataset);
        }
        let sofa = OpenOptions::new()
            .sample_rate(sample_rate as f32)
            .normalized(false)
            .open_data(data)
            .map_err(SofaError::Decode)?;
        if sofa.hrtf().r() != 2
            || !(1..=MAX_FILTER_FRAMES).contains(&sofa.filter_len())
            || !(1..=32_768).contains(&sofa.num_measurements())
            || !(0. ..=0.04).contains(&sofa.max_delay())
        {
            return Err(SofaError::UnsupportedDataset);
        }
        Ok(Self {
            sofa,
            rate: sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    pub fn measurements(&self) -> u32 {
        self.sofa.num_measurements()
    }

    /// Interpolate the HRTFs and fold the SOFA per-ear delay into two FIRs.
    pub fn impulse(&self, direction: Direction) -> Result<StereoImpulse, Error> {
        let [x, y, z] = direction.cartesian()?;
        let mut filter = Filter::new(self.sofa.filter_len());
        self.sofa.filter(x, y, z, &mut filter);
        if filter.left.iter().chain(&filter.right).all(|&v| v == 0.) {
            return Err(Error::InvalidBuffer);
        }
        pack_delays(&filter, self.rate)
    }
}

fn pack_delays(filter: &Filter, rate: u32) -> Result<StereoImpulse, Error> {
    let l = filter.ldelay * rate as f32;
    let r = filter.rdelay * rate as f32;
    let cap = 0.04 * rate as f32 + 1.;
    if !(0. ..=cap).contains(&l)
        || !(0. ..=cap).contains(&r)
        || filter.left.len() != filter.right.len()
        || filter.left.is_empty()
    {
        return Err(Error::InvalidParameter);
    }
    let frames = filter.left.len() + l.max(r).floor() as usize + 1;
    let mut data = vec![0.; 2 * frames];
    for (channel, ir, delay) in [(0, &filter.left, l), (1, &filter.right, r)] {
        let offset = delay.floor() as usize;
        let fraction = delay.fract();
        for (i, &sample) in ir.iter().enumerate() {
            data[2 * (offset + i) + channel] += sample * (1. - fraction);
            if fraction != 0. {
                data[2 * (offset + i + 1) + channel] += sample * fraction;
            }
        }
    }
    StereoImpulse::new(data, 2, f64::from(rate))
}

pub struct BinauralControl {
    convolution: ConvolutionController,
    rate: u32,
}
impl BinauralControl {
    /// Build and enqueue a replacement. While the queue is full, retry later.
    pub fn set_direction(&mut self, dataset: &HrtfSet, direction: Direction) -> Result<(), Error> {
        if dataset.rate != self.rate {
            return Err(Error::InvalidSpec);
        }
        self.convolution.replace(dataset.impulse(direction)?)
    }

    pub fn collect_retired(&mut self) -> usize {
        self.convolution.collect_retired()
    }
}

pub struct Binaural {
    convolution: ReloadableConvolution,
    max_frames: usize,
}
impl Binaural {
    pub fn prepare(
        dataset: &HrtfSet,
        direction: Direction,
        max_frames: usize,
        fade_frames: usize,
    ) -> Result<(BinauralControl, Self), Error> {
        let spec = ProcessSpec {
            sample_rate: f64::from(dataset.rate),
            max_block_frames: max_frames,
            channels: 2,
        };
        let (controller, processor) = convolution_pair(
            dataset.impulse(direction)?,
            ConvolutionMode::ZeroLatency,
            0,
            fade_frames,
            spec,
        )?;
        Ok((
            BinauralControl {
                convolution: controller,
                rate: dataset.rate,
            },
            Self {
                convolution: processor,
                max_frames,
            },
        ))
    }

    /// Render a mono source to an interleaved stereo headphone buffer.
    pub fn process_mono(
        &mut self,
        source: &[f32],
        stereo: &mut AudioBuffer<'_>,
    ) -> Result<(), Error> {
        if stereo.channels() != 2
            || stereo.frames() != source.len()
            || source.len() > self.max_frames
        {
            return Err(Error::InvalidBuffer);
        }
        for (dst, &x) in stereo
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(source)
        {
            dst.fill(if x.is_finite() {
                x.clamp(-16., 16.)
            } else {
                0.
            });
        }
        self.convolution.process(
            stereo,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        Ok(())
    }

    pub fn latency_frames(&self) -> usize {
        self.convolution.latency_frames()
    }
    pub fn reset(&mut self) {
        self.convolution.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_fraction_and_ear_order_are_preserved() {
        let spec = ProcessSpec {
            sample_rate: 48_000.,
            max_block_frames: 64,
            channels: 2,
        };
        let mut filter = Filter::new(1);
        filter.left[0] = 1.;
        filter.right[0] = 0.5;
        filter.ldelay = 2.25 / 48_000.;
        filter.rdelay = 4. / 48_000.;
        let (_, convolution) = convolution_pair(
            pack_delays(&filter, 48_000).unwrap(),
            ConvolutionMode::ZeroLatency,
            0,
            32,
            spec,
        )
        .unwrap();
        let mut dsp = Binaural {
            convolution,
            max_frames: 64,
        };
        let mut x = [0.; 16];
        x[0] = 1.;
        let mut result = [0.; 32];
        let allocations = allocation_counter::measure(|| {
            dsp.process_mono(&x, &mut AudioBuffer::new(&mut result, 2).unwrap())
                .unwrap()
        });
        assert_eq!(allocations.count_total, 0);
        assert!((result[4] - 0.75).abs() < 1e-6);
        assert!((result[6] - 0.25).abs() < 1e-6);
        assert!((result[9] - 0.5).abs() < 1e-6);
    }
}
