//! Cascaded two-phase half-band IIR conversion.
use crate::oversampling::Quality;
use soniclaw_core::{AudioBuffer, Error, ProcessSpec};

// Balanced coefficients fitted on the normalized 0.30–0.50 stopband.
const EVEN: &[f64] = &[0.037169042500422, 0.29228376549031, 0.66545164172295];
const ODD: &[f64] = &[0.14091691346082, 0.47103815351315, 0.87912961015299];
// Four sections per branch, fitted on the wider 0.28–0.50 stopband.
const HIGH_EVEN: &[f64] = &[
    0.030211625586495382,
    0.23775868703652048,
    0.5264831852245073,
    0.7998933102222173,
];
const HIGH_ODD: &[f64] = &[
    0.1147076844572862,
    0.38035212580537925,
    0.6668670852198956,
    0.9312328457381965,
];

pub(crate) fn allpass(state: &mut f64, input: f64, coefficient: f64) -> f64 {
    let output = coefficient * input + *state;
    *state = input - coefficient * output;
    output
}

#[derive(Default, Clone, Copy)]
pub(crate) struct AllpassDelay(f64);
impl AllpassDelay {
    pub(crate) fn tick(&mut self, input: f64, fractional_delay: f64) -> f64 {
        if fractional_delay <= 1e-8 {
            self.0 = 0.;
            return input;
        }
        let coefficient = (1. - fractional_delay) / (1. + fractional_delay);
        allpass(&mut self.0, input, coefficient)
    }
}

#[derive(Default, Clone)]
struct Branch([f64; 4]);
impl Branch {
    fn tick(&mut self, mut input: f64, coefficients: &[f64]) -> f64 {
        for (state, &a) in self.0.iter_mut().zip(coefficients) {
            input = allpass(state, input, a);
        }
        input
    }
}

#[derive(Default, Clone)]
struct Channel {
    up_a: Branch,
    up_b: Branch,
    down_a: Branch,
    down_b: Branch,
    previous_odd: f64,
}

pub struct IirOversampler {
    spec: ProcessSpec,
    high: ProcessSpec,
    work: Vec<f32>,
    scratch: Vec<f32>,
    stages: Vec<Vec<Channel>>,
    even: &'static [f64],
    odd: &'static [f64],
}
impl IirOversampler {
    pub fn new(spec: ProcessSpec) -> Result<Self, Error> {
        Self::with_stages(spec, 1, Quality::Balanced, 2_097_152)
    }

    pub(crate) fn with_stages(
        spec: ProcessSpec,
        stages: usize,
        quality: Quality,
        limit: usize,
    ) -> Result<Self, Error> {
        spec.validate()?;
        let ratio = 1 << stages;
        let count = (spec.max_block_frames * spec.channels)
            .checked_mul(ratio)
            .filter(|&count| count <= limit)
            .ok_or(Error::InvalidSpec)?;
        let high = ProcessSpec {
            sample_rate: spec.sample_rate * ratio as f64,
            max_block_frames: spec.max_block_frames * ratio,
            channels: spec.channels,
        }
        .validate()?;
        let (even, odd) = match quality {
            Quality::Balanced => (EVEN, ODD),
            Quality::High => (HIGH_EVEN, HIGH_ODD),
        };
        Ok(Self {
            spec,
            high,
            work: vec![0.; count],
            scratch: vec![0.; if stages > 1 { count } else { 0 }],
            stages: (0..stages)
                .map(|_| vec![Channel::default(); spec.channels])
                .collect(),
            even,
            odd,
        })
    }

    pub fn high_rate_spec(&self) -> ProcessSpec {
        self.high
    }

    /// Low-frequency group delay, expressed at the original rate.
    pub fn dc_delay_samples(&self) -> f64 {
        let stage = 0.5
            + self
                .even
                .iter()
                .chain(self.odd)
                .map(|a| (1. - a) / (1. + a))
                .sum::<f64>();
        (0..self.stages.len())
            .map(|i| stage / (1 << i) as f64)
            .sum()
    }

    pub fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.fill(Channel::default());
        }
        self.work.fill(0.);
        self.scratch.fill(0.);
    }

    pub fn process_with(
        &mut self,
        block: &mut AudioBuffer<'_>,
        render: impl FnOnce(&mut AudioBuffer<'_>, ProcessSpec),
    ) -> Result<(), Error> {
        let channels = self.spec.channels;
        if block.channels() != channels
            || block.frames() > self.spec.max_block_frames
            || block.samples().iter().any(|x| !x.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        let mut samples = block.samples().len();
        if samples == 0 {
            return Ok(());
        }
        up(
            block.samples(),
            &mut self.work[..2 * samples],
            &mut self.stages[0],
            self.even,
            self.odd,
        );
        samples *= 2;
        for stage in &mut self.stages[1..] {
            up(
                &self.work[..samples],
                &mut self.scratch[..2 * samples],
                stage,
                self.even,
                self.odd,
            );
            std::mem::swap(&mut self.work, &mut self.scratch);
            samples *= 2;
        }
        render(
            &mut AudioBuffer::new(&mut self.work[..samples], channels)?,
            self.high,
        );
        if self.work[..samples].iter().any(|x| !x.is_finite()) {
            block.clear();
            self.reset();
            return Err(Error::InvalidBuffer);
        }
        for stage in self.stages[1..].iter_mut().rev() {
            down(
                &self.work[..samples],
                &mut self.scratch[..samples / 2],
                stage,
                self.even,
                self.odd,
            );
            std::mem::swap(&mut self.work, &mut self.scratch);
            samples /= 2;
        }
        down(
            &self.work[..samples],
            block.samples_mut(),
            &mut self.stages[0],
            self.even,
            self.odd,
        );
        Ok(())
    }
}

fn up(input: &[f32], output: &mut [f32], states: &mut [Channel], even: &[f64], odd: &[f64]) {
    let channels = states.len();
    for (frame, pair) in input
        .chunks_exact(channels)
        .zip(output.chunks_exact_mut(2 * channels))
    {
        let (a, b) = pair.split_at_mut(channels);
        for ((&x, state), (out_a, out_b)) in frame.iter().zip(&mut *states).zip(a.iter_mut().zip(b))
        {
            *out_a = state.up_a.tick(f64::from(x), even) as f32;
            *out_b = state.up_b.tick(f64::from(x), odd) as f32;
        }
    }
}

fn down(input: &[f32], output: &mut [f32], states: &mut [Channel], even: &[f64], odd: &[f64]) {
    let channels = states.len();
    for (frame, pair) in output
        .chunks_exact_mut(channels)
        .zip(input.chunks_exact(2 * channels))
    {
        let (a, b) = pair.split_at(channels);
        for ((out, state), (&x_a, &x_b)) in frame.iter_mut().zip(&mut *states).zip(a.iter().zip(b))
        {
            let value = state.down_a.tick(f64::from(x_a), even);
            *out = (0.5 * (value + state.previous_odd)) as f32;
            state.previous_odd = state.down_b.tick(f64::from(x_b), odd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dc_partition_and_rejection() {
        let spec = ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 1024,
        };
        let run = |block: usize| {
            let mut o = IirOversampler::new(spec).unwrap();
            let mut x = vec![1.; 1024];
            for b in x.chunks_mut(block) {
                o.process_with(&mut AudioBuffer::new(b, 1).unwrap(), |_, _| {})
                    .unwrap();
            }
            x
        };
        let a = run(1024);
        assert_eq!(a, run(37));
        assert!((a[1023] - 1.).abs() < 1e-5);
        let mut o = IirOversampler::new(spec).unwrap();
        let mut x = vec![0.; 1024];
        let mut t = 0.;
        o.process_with(&mut AudioBuffer::new(&mut x, 1).unwrap(), |high, _| {
            for y in high.samples_mut() {
                *y = (t * std::f32::consts::TAU * 0.4).sin();
                t += 1.;
            }
        })
        .unwrap();
        let rms = (x[512..].iter().map(|x| x * x).sum::<f32>() / 512.).sqrt();
        assert!(rms < 0.001, "{rms}");
    }
}
