//! First-order topology-preserving filters with independent channel state.
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    LowPass,
    HighPass,
    AllPass,
}

/// A one-pole topology-preserving transform of an analog integrator.
pub struct FirstOrderTpt {
    mode: Mode,
    cutoff_hz: f64,
    coefficient: f64,
    spec: Option<ProcessSpec>,
    memories: Vec<f64>,
}

impl FirstOrderTpt {
    pub fn new(mode: Mode, cutoff_hz: f64) -> Result<Self, Error> {
        if !cutoff_hz.is_finite() || cutoff_hz <= 0. {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            mode,
            cutoff_hz,
            coefficient: 0.,
            spec: None,
            memories: Vec::new(),
        })
    }

    /// The mode changes the output tap and keeps the state of the integrator.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f64) -> Result<(), Error> {
        if !cutoff_hz.is_finite() || cutoff_hz <= 0. {
            return Err(Error::InvalidParameter);
        }
        if let Some(s) = self.spec {
            self.coefficient = coefficient(cutoff_hz, s.sample_rate)?;
        }
        self.cutoff_hz = cutoff_hz;
        Ok(())
    }

    /// The slice holds one cutoff in Hz per frame, shared across channels.
    /// Malformed input is rejected before processing any frames or state.
    pub fn process_modulated(
        &mut self,
        buffer: &mut AudioBuffer<'_>,
        cutoff_hz: &[f32],
    ) -> Result<(), Error> {
        let s = self.check_buffer(buffer)?;
        if cutoff_hz.len() != buffer.frames()
            || cutoff_hz
                .iter()
                .any(|&f| !valid_cutoff(f64::from(f), s.sample_rate))
        {
            return Err(Error::InvalidParameter);
        }
        for (frame, &hz) in buffer
            .samples_mut()
            .chunks_exact_mut(s.channels)
            .zip(cutoff_hz)
        {
            let g = (PI * f64::from(hz) / s.sample_rate).tan();
            tick(self.mode, g / (1. + g), &mut self.memories, frame);
        }
        Ok(())
    }

    fn check_buffer(&self, buffer: &AudioBuffer<'_>) -> Result<ProcessSpec, Error> {
        let s = self.spec.ok_or(Error::NotPrepared)?;
        if buffer.channels() != s.channels {
            return Err(Error::InvalidBuffer);
        }
        if buffer.frames() > s.max_block_frames {
            return Err(Error::BlockTooLarge);
        }
        Ok(s)
    }
}

fn valid_cutoff(hz: f64, rate: f64) -> bool {
    hz > 0. && hz <= 0.49 * rate
}

fn coefficient(hz: f64, rate: f64) -> Result<f64, Error> {
    if !valid_cutoff(hz, rate) {
        return Err(Error::InvalidParameter);
    }
    let g = (PI * hz / rate).tan();
    Ok(g / (1. + g))
}

fn tick(mode: Mode, coefficient: f64, memories: &mut [f64], frame: &mut [f32]) {
    for (sample, memory) in frame.iter_mut().zip(memories) {
        if !sample.is_finite() {
            *sample = 0.;
            *memory = 0.;
            continue;
        }
        let x = f64::from(*sample);
        let correction = (x - *memory) * coefficient;
        let low = *memory + correction;
        *memory = low + correction;
        if memory.abs() < 1e-40 {
            *memory = 0.;
        }
        let output = match mode {
            Mode::LowPass => low,
            Mode::HighPass => x - low,
            Mode::AllPass => 2. * low - x,
        };
        *sample = output.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
    }
}

impl AudioProcessor for FirstOrderTpt {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let s = spec.validate()?;
        self.coefficient = coefficient(self.cutoff_hz, s.sample_rate)?;
        self.memories = vec![0.; s.channels];
        self.spec = Some(s);
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Ok(s) = self.check_buffer(buffer) else {
            buffer.clear();
            return;
        };
        for frame in buffer.samples_mut().chunks_exact_mut(s.channels) {
            tick(self.mode, self.coefficient, &mut self.memories, frame);
        }
    }

    fn reset(&mut self) {
        self.memories.fill(0.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::Transport;
    const SPEC: ProcessSpec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 129,
        channels: 2,
    };

    #[test]
    fn complementary_taps_and_allpass_energy() {
        let impulse = |mode| {
            let mut engine = FirstOrderTpt::new(mode, 1000.).unwrap();
            engine.prepare(SPEC).unwrap();
            let mut signal = vec![0.; 1000];
            signal[0] = 1.;
            let ctx = ProcessContext {
                events: &[],
                transport: Transport::default(),
            };
            for part in signal.chunks_mut(128) {
                engine.process(&mut AudioBuffer::new(part, 2).unwrap(), &ctx);
            }
            assert!(signal.iter().skip(1).step_by(2).all(|&x| x == 0.));
            signal
        };
        let low = impulse(Mode::LowPass);
        let high = impulse(Mode::HighPass);
        for (i, (&x, y)) in low.iter().zip(high).enumerate() {
            assert!((x + y - if i == 0 { 1. } else { 0. }).abs() < 1e-7);
        }
        let energy: f64 = impulse(Mode::AllPass)
            .into_iter()
            .map(|x| f64::from(x).powi(2))
            .sum();
        assert!((energy - 1.).abs() < 1e-6);
    }

    #[test]
    fn modulation_is_partition_independent_and_invalid_cutoff_is_transactional() {
        let input: Vec<_> = (0..2000).map(|i| (i as f32 * 0.057).sin()).collect();
        let hz: Vec<_> = (0..1000)
            .map(|i| 20. + 23100. * (i as f32 * 0.397).sin().abs())
            .collect();
        let mut filter = FirstOrderTpt::new(Mode::HighPass, 200.).unwrap();
        filter.prepare(SPEC).unwrap();
        let process = |engine: &mut FirstOrderTpt, size| {
            let mut audio = input.clone();
            for (b, h) in audio.chunks_mut(size * 2).zip(hz.chunks(size)) {
                engine
                    .process_modulated(&mut AudioBuffer::new(b, 2).unwrap(), h)
                    .unwrap();
            }
            audio
        };
        let first = process(&mut filter, 129);
        filter.reset();
        let mut rejected = [0.6; 4];
        assert!(filter
            .process_modulated(
                &mut AudioBuffer::new(&mut rejected, 2).unwrap(),
                &[500., 24000.]
            )
            .is_err());
        assert_eq!(rejected, [0.6; 4]);
        assert_eq!(first, process(&mut filter, 17));
    }
}
