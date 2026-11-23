//! Native double-precision gain, filters and linked dynamics.
use crate::{dynamics, filter_design};
use soniclaw_core::{AudioBuffer64, AudioProcessor64, Error, ProcessContext, ProcessSpec};

pub(crate) trait Sample: Copy {
    fn to_f64(self) -> f64;
    fn from_f64(sample: f64) -> Self;
}
impl Sample for f32 {
    fn to_f64(self) -> f64 {
        f64::from(self)
    }
    fn from_f64(sample: f64) -> Self {
        sample as f32
    }
}
impl Sample for f64 {
    fn to_f64(self) -> f64 {
        self
    }
    fn from_f64(sample: f64) -> Self {
        sample
    }
}

pub struct Gain64 {
    spec: Option<ProcessSpec>,
    current: f64,
    target: f64,
    step: f64,
    remaining: usize,
}

impl Gain64 {
    pub fn new(gain: f64) -> Result<Self, Error> {
        let mut state = Self {
            spec: None,
            current: 0.,
            target: 0.,
            step: 0.,
            remaining: 0,
        };
        state.set_gain(gain, 0)?;
        Ok(state)
    }

    /// Set a linear-amplitude target using the same gain on every channel of a frame.
    pub fn set_gain(&mut self, gain: f64, ramp_frames: usize) -> Result<(), Error> {
        if !(-64. ..=64.).contains(&gain) || ramp_frames > 16_777_216 {
            return Err(Error::InvalidParameter);
        }
        self.target = gain;
        self.remaining = ramp_frames;
        if ramp_frames == 0 {
            self.current = gain;
        } else {
            self.step = (gain - self.current) / ramp_frames as f64;
        }
        Ok(())
    }
}

impl AudioProcessor64 for Gain64 {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.spec = Some(spec.validate()?);
        self.reset();
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer64<'_>, _: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            buffer.clear();
            return;
        };
        if buffer.frames() > spec.max_block_frames || buffer.channels() != spec.channels {
            buffer.clear();
            return;
        }
        for frame in buffer.samples_mut().chunks_exact_mut(spec.channels) {
            if self.remaining > 0 {
                self.remaining -= 1;
                self.current = if self.remaining == 0 {
                    self.target
                } else {
                    self.current + self.step
                };
            }
            for sample in frame {
                *sample *= self.current;
            }
        }
    }
    fn reset(&mut self) {
        self.current = self.target;
        self.remaining = 0;
    }
}
pub use dynamics::Dynamics64;
pub use filter_design::Filter64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter_design::Section;
    use soniclaw_core::{AudioBuffer, AudioProcessor};

    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48_000.,
            channels: 2,
            max_block_frames: 64,
        }
    }
    fn context() -> ProcessContext<'static> {
        ProcessContext {
            events: &[],
            transport: Default::default(),
        }
    }

    #[test]
    fn native_gain_and_filters_preserve_sub_f32_details() {
        let s = spec();
        let epsilon = 2f64.powi(-38);
        let mut gain = Gain64::new(1.).unwrap();
        gain.prepare(s).unwrap();
        gain.set_gain(2., 2).unwrap();
        let mut signal = [1. + epsilon, 0., 1. + epsilon, 0.];
        gain.process(&mut AudioBuffer64::new(&mut signal, 2).unwrap(), &context());
        assert_eq!(signal, [1.5 * (1. + epsilon), 0., 2. * (1. + epsilon), 0.]);
        let mut filters = [
            Filter64::fir(s.sample_rate, vec![0.25, 0.5, 0.25]).unwrap(),
            Filter64::iir(
                s.sample_rate,
                vec![Section::new([0.5, 0., 0.], [1., -0.5, 0.]).unwrap()],
            )
            .unwrap(),
        ];
        for filter in &mut filters {
            filter.prepare(s).unwrap();
            let mut pcm = [1. + epsilon, 0., 0., 0., 0., 0.];
            let allocations = allocation_counter::measure(|| {
                filter.process(&mut AudioBuffer64::new(&mut pcm, 2).unwrap(), &context());
            });
            assert_eq!(allocations.count_total, 0);
            assert_ne!(pcm[0], f64::from(pcm[0] as f32));
            assert!(pcm.iter().skip(1).step_by(2).all(|v| *v == 0.));
        }
    }

    #[test]
    fn double_lookahead_is_precise_and_f32_entry_point_is_equivalent() {
        let s = spec();
        let config = dynamics::Config {
            lookahead_ms: 1.,
            controls: dynamics::Controls {
                ratio: 1.,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut f32_processor = dynamics::Dynamics::new(config).unwrap();
        let mut f64_processor = Dynamics64::new(config).unwrap();
        f32_processor.prepare(s).unwrap();
        f64_processor.prepare(s).unwrap();
        let mut a = [0.; 128];
        let mut b = [0.; 128];
        a[0] = 1.;
        b[0] = 1. + 2f64.powi(-38);
        let measure = allocation_counter::measure(|| {
            f64_processor.process(&mut AudioBuffer64::new(&mut b, 2).unwrap(), &context());
        });
        assert_eq!(measure.count_total, 0);
        f32_processor.process(&mut AudioBuffer::new(&mut a, 2).unwrap(), &context());
        assert_eq!(b[96], 1. + 2f64.powi(-38));
        assert_eq!(b.map(|v| v as f32), a);
        assert_eq!(f64_processor.latency_frames(), 48);
    }

    #[test]
    fn keyed_double_dynamics_tracks_legacy_results_across_partitions() {
        let config = dynamics::Config {
            detector: dynamics::Detector::Rms { window_frames: 17 },
            key_highpass_hz: 100.,
            ..Default::default()
        };
        let mut a = dynamics::Dynamics::new(config).unwrap();
        let mut b = Dynamics64::new(config).unwrap();
        a.prepare(spec()).unwrap();
        b.prepare(spec()).unwrap();
        let mut f32_signal: Vec<_> = (0..512).map(|i| (i as f32 * 0.031).sin()).collect();
        let mut f64_signal: Vec<_> = f32_signal.iter().map(|&sample| f64::from(sample)).collect();
        let key: Vec<_> = (0..256).map(|i| (i as f32 * 0.21).sin()).collect();
        let key64: Vec<_> = key.iter().map(|&sample| f64::from(sample)).collect();
        let mut offset = 0;
        for chunk in f32_signal.chunks_mut(58) {
            let length = chunk.len() / 2;
            a.process_keyed(
                &mut AudioBuffer::new(chunk, 2).unwrap(),
                &key[offset..offset + length],
                1,
            )
            .unwrap();
            offset += length;
        }
        offset = 0;
        for chunk in f64_signal.chunks_mut(94) {
            let length = chunk.len() / 2;
            b.process_keyed(
                &mut AudioBuffer64::new(chunk, 2).unwrap(),
                &key64[offset..offset + length],
                1,
            )
            .unwrap();
            offset += length;
        }
        assert!(f64_signal
            .iter()
            .zip(f32_signal)
            .all(|(&value, expected)| value as f32 == expected));
    }
}
