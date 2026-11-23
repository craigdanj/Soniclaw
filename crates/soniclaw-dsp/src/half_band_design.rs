//! Specification-driven half-band FIR and polyphase IIR oversampling.
use crate::iir_oversampling::allpass;
use soniclaw_core::{AudioBuffer, Error, ProcessSpec};
use std::f64::consts::{PI, TAU};

const GRID: usize = 4096;
const MAX_TERMS: usize = 64;

/// Width is the full transition interval in cycles per high-rate sample.
/// The passband ends at 0.25 - width/2 and the stopband starts at 0.25 + width/2.
#[derive(Debug, Clone, Copy)]
pub struct FilterSpec {
    pub transition_width: f64,
    pub attenuation_db: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum Family {
    EquirippleFir,
    PolyphaseIir,
}

#[derive(Debug, Clone, Copy)]
pub struct StageSpec {
    pub family: Family,
    pub up: FilterSpec,
    pub down: FilterSpec,
}

#[derive(Debug, Clone)]
enum Coefficients {
    Fir(Vec<f64>),
    Allpass([Vec<f64>; 2]),
}

/// Prepared coefficients for a unit-gain half-band low-pass filter.
#[derive(Debug, Clone)]
pub struct HalfBand {
    coefficients: Coefficients,
    stopband_peak: f64,
}

impl HalfBand {
    /// Design and check the response on a dense frequency grid. Call off the audio thread.
    pub fn design(family: Family, spec: FilterSpec) -> Result<Self, Error> {
        let width = spec.transition_width;
        if !(0.02..=0.24).contains(&width) || !(20.0..=120.0).contains(&spec.attenuation_db) {
            return Err(Error::InvalidParameter);
        }
        let edge = 0.25 + width * 0.5;
        let limit = 10f64.powf(-spec.attenuation_db / 20.0);
        let start = match family {
            Family::EquirippleFir => ((spec.attenuation_db - 12.) / (64. * width)).ceil() as usize,
            Family::PolyphaseIir => 1,
        };
        for order in start.max(1)..=MAX_TERMS {
            let coefficients = match family {
                Family::EquirippleFir => {
                    let Some(taps) = fir_exchange(order, width) else {
                        continue;
                    };
                    Coefficients::Fir(taps)
                }
                Family::PolyphaseIir => Coefficients::Allpass(elliptic(order, width)),
            };
            let mut filter = Self {
                coefficients,
                stopband_peak: 0.,
            };
            filter.stopband_peak = (0..=GRID)
                .map(|i| {
                    filter
                        .magnitude_at(edge + (0.5 - edge) * i as f64 / GRID as f64)
                        .unwrap()
                })
                .fold(0., f64::max);
            if filter.stopband_peak <= limit * 0.999 {
                return Ok(filter);
            }
        }
        Err(Error::InvalidParameter)
    }

    pub fn fir_taps(&self) -> Option<&[f64]> {
        match &self.coefficients {
            Coefficients::Fir(taps) => Some(taps),
            Coefficients::Allpass(_) => None,
        }
    }

    /// Returns the even and odd branch coefficients at the lower rate.
    pub fn allpass_branches(&self) -> Option<[&[f64]; 2]> {
        match &self.coefficients {
            Coefficients::Fir(_) => None,
            Coefficients::Allpass([a, b]) => Some([a, b]),
        }
    }

    pub fn measured_attenuation_db(&self) -> f64 {
        -20. * self.stopband_peak.log10()
    }

    /// Frequency is in cycles per high-rate sample, from DC to Nyquist.
    pub fn magnitude_at(&self, frequency: f64) -> Result<f64, Error> {
        if !(0. ..=0.5).contains(&frequency) {
            return Err(Error::InvalidParameter);
        }
        let angle = TAU * frequency;
        Ok(match &self.coefficients {
            Coefficients::Fir(taps) => {
                let center = taps.len() / 2;
                (0.5 + 2.
                    * (1..=center)
                        .step_by(2)
                        .map(|d| taps[center + d] * (d as f64 * angle).cos())
                        .sum::<f64>())
                .abs()
            }
            Coefficients::Allpass([a, b]) => {
                // An allpass section has unit magnitude. The average of two
                // unit vectors has magnitude |cos(half their phase difference)|.
                let low_rate_tan = angle.tan();
                let branch = |coefficients: &[f64]| {
                    coefficients
                        .iter()
                        .map(|c| -2. * (low_rate_tan * (1. - c) / (1. + c)).atan())
                        .sum::<f64>()
                };
                (0.5 * (branch(a) - branch(b) + angle)).cos().abs()
            }
        })
    }

    fn delay_at_lower_rate(&self) -> f64 {
        match &self.coefficients {
            Coefficients::Fir(taps) => (taps.len() / 2) as f64 * 0.5,
            Coefficients::Allpass(branches) => {
                0.25 + 0.5
                    * branches
                        .iter()
                        .flatten()
                        .map(|a| (1. - a) / (1. + a))
                        .sum::<f64>()
            }
        }
    }
}

// Remez exchange on the passband of the odd-cosine half-band basis.
// Symmetry automatically gives an equal stopband ripple and exact zero taps.
fn fir_exchange(terms: usize, width: f64) -> Option<Vec<f64>> {
    let step = TAU * (0.25 - width * 0.5) / GRID as f64;
    let basis: Vec<Vec<f64>> = (0..=GRID)
        .map(|i| {
            (0..terms)
                .map(|j| (((2 * j + 1) * i) as f64 * step).cos())
                .collect()
        })
        .collect();
    let mut extrema: Vec<_> = (0..=terms).map(|j| j * GRID / terms).collect();
    for _ in 0..48 {
        let mut matrix: Vec<_> = extrema
            .iter()
            .enumerate()
            .map(|(j, &i)| {
                let mut row = basis[i].clone();
                row.extend([if j % 2 == 0 { 1. } else { -1. }, 0.5]);
                row
            })
            .collect();
        let solution = solve(&mut matrix)?;
        let error: Vec<f64> = basis
            .iter()
            .map(|row| row.iter().zip(&solution).map(|(a, b)| a * b).sum::<f64>() - 0.5)
            .collect();
        let ripple = solution[terms].abs();
        if ripple > 0. && error.iter().all(|e| e.abs() <= ripple * 1.001 + 1e-13) {
            let center = 2 * terms - 1;
            let mut taps = vec![0.; 2 * center + 1];
            taps[center] = 0.5;
            for (j, coefficient) in solution[..terms].iter().enumerate() {
                let distance = 2 * j + 1;
                taps[center + distance] = 0.5 * coefficient;
                taps[center - distance] = 0.5 * coefficient;
            }
            return Some(taps);
        }
        let mut next: Vec<usize> = Vec::new();
        for i in 0..=GRID {
            let value = error[i];
            let sign = if value >= 0. { 1. } else { -1. };
            if (i > 0 && sign * error[i - 1] > value.abs())
                || (i < GRID && sign * error[i + 1] > value.abs())
            {
                continue;
            }
            if let Some(last) = next
                .last_mut()
                .filter(|last| error[**last].is_sign_positive() == value.is_sign_positive())
            {
                if value.abs() > error[*last].abs() {
                    *last = i;
                }
            } else {
                next.push(i);
            }
        }
        while next.len() > terms + 1 {
            if error[next[0]].abs() < error[*next.last()?].abs() {
                next.remove(0);
            } else {
                next.pop();
            }
        }
        if next.len() != terms + 1 || extrema == next {
            return None;
        }
        extrema = next;
    }
    None
}

fn solve(matrix: &mut [Vec<f64>]) -> Option<Vec<f64>> {
    let n = matrix.len();
    for i in 0..n {
        let pivot = (i..n).max_by(|&a, &b| matrix[a][i].abs().total_cmp(&matrix[b][i].abs()))?;
        matrix.swap(i, pivot);
        if matrix[i][i].abs() < 1e-14 {
            return None;
        }
        let (head, tail) = matrix.split_at_mut(i + 1);
        let row = &head[i];
        for lower in tail {
            let ratio = lower[i] / row[i];
            for j in i..=n {
                lower[j] -= ratio * row[j];
            }
        }
    }
    let mut out = vec![0.; n];
    for i in (0..n).rev() {
        let remainder = (i + 1..n).map(|j| matrix[i][j] * out[j]).sum::<f64>();
        out[i] = (matrix[i][n] - remainder) / matrix[i][i];
    }
    Some(out)
}

fn agm(mut a: f64, mut b: f64) -> f64 {
    for _ in 0..16 {
        if (a - b).abs() < 1e-15 {
            break;
        }
        (a, b) = ((a + b) * 0.5, (a * b).sqrt());
    }
    (a + b) * 0.5
}

fn elliptic(sections: usize, width: f64) -> [Vec<f64>; 2] {
    let k = ((1. - 2. * width) * PI * 0.25).tan().powi(2);
    let q = (-PI * agm(1., (1. - k * k).sqrt()) / agm(1., k)).exp();
    let mut branches = [Vec::new(), Vec::new()];
    for j in 1..=sections {
        let theta = j as f64 * PI / (2 * sections + 1) as f64;
        let numerator: f64 = (0..12)
            .map(|i| {
                (-1f64).powi(i)
                    * q.powf((i * (i + 1)) as f64 + 0.25)
                    * ((2 * i + 1) as f64 * theta).sin()
            })
            .sum();
        let denominator: f64 = 1.
            + 2. * (1..12)
                .map(|i| (-1f64).powi(i) * q.powi(i * i) * ((2 * i) as f64 * theta).cos())
                .sum::<f64>();
        let a = (2. * numerator / denominator).powi(2);
        let x = ((1. - a * k) * (1. - a / k)).max(0.).sqrt() / (1. + a);
        branches[(j - 1) % 2].push((1. - x) / (1. + x));
    }
    branches
}

struct History {
    data: Vec<f64>,
    position: usize,
}

impl History {
    fn new(length: usize) -> Self {
        Self {
            data: vec![0.; length],
            position: 0,
        }
    }

    fn feed(&mut self, x: f64) {
        self.position = (self.position + 1) % self.data.len();
        self.data[self.position] = x;
    }

    fn delayed(&self, offset: usize) -> f64 {
        self.data[(self.position + self.data.len() - offset) % self.data.len()]
    }

    fn reset(&mut self) {
        self.data.fill(0.);
        self.position = 0;
    }
}

enum Direction {
    Fir {
        taps: Vec<f64>,
        histories: Vec<History>,
    },
    Allpass {
        coefficients: [Vec<f64>; 2],
        histories: Vec<[Vec<f64>; 2]>,
        odd: Vec<f64>,
    },
}

impl Direction {
    fn new(design: HalfBand, channels: usize, up: bool) -> Self {
        match design.coefficients {
            Coefficients::Fir(taps) => {
                let length = if up {
                    taps.len().div_ceil(2)
                } else {
                    taps.len()
                };
                Self::Fir {
                    taps,
                    histories: (0..channels).map(|_| History::new(length)).collect(),
                }
            }
            Coefficients::Allpass(coefficients) => {
                let histories = vec![
                    [
                        vec![0.; coefficients[0].len()],
                        vec![0.; coefficients[1].len()]
                    ];
                    channels
                ];
                Self::Allpass {
                    coefficients,
                    histories,
                    odd: vec![0.; channels],
                }
            }
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Fir { histories, .. } => histories.iter_mut().for_each(History::reset),
            Self::Allpass { histories, odd, .. } => {
                odd.fill(0.);
                histories.iter_mut().flatten().for_each(|s| s.fill(0.));
            }
        }
    }

    fn up(&mut self, input: &[f32], output: &mut [f32], channels: usize) {
        for (frame, pair) in input
            .chunks_exact(channels)
            .zip(output.chunks_exact_mut(2 * channels))
        {
            let (even, odd) = pair.split_at_mut(channels);
            for (c, ((a, b), &x)) in even.iter_mut().zip(odd).zip(frame).enumerate() {
                let (y, z) = match self {
                    Self::Fir { taps, histories } => {
                        let history = &mut histories[c];
                        history.feed(f64::from(x));
                        let low = 2.
                            * taps
                                .iter()
                                .step_by(2)
                                .enumerate()
                                .map(|(i, t)| t * history.delayed(i))
                                .sum::<f64>();
                        (low, history.delayed((taps.len() / 2) / 2))
                    }
                    Self::Allpass {
                        coefficients,
                        histories,
                        ..
                    } => {
                        let [s, t] = &mut histories[c];
                        (
                            cascade(s, &coefficients[0], f64::from(x)),
                            cascade(t, &coefficients[1], f64::from(x)),
                        )
                    }
                };
                *a = y as f32;
                *b = z as f32;
            }
        }
    }

    fn down(&mut self, input: &[f32], output: &mut [f32], channels: usize) {
        for (frame, pair) in output
            .chunks_exact_mut(channels)
            .zip(input.chunks_exact(2 * channels))
        {
            let (a, b) = pair.split_at(channels);
            for (c, (out, (&x, &y))) in frame.iter_mut().zip(a.iter().zip(b)).enumerate() {
                let value = match self {
                    Self::Fir { taps, histories } => {
                        let history = &mut histories[c];
                        history.feed(f64::from(x));
                        let center = taps.len() / 2;
                        let value = taps
                            .iter()
                            .step_by(2)
                            .enumerate()
                            .map(|(i, t)| t * history.delayed(2 * i))
                            .sum::<f64>()
                            + 0.5 * history.delayed(center);
                        history.feed(f64::from(y));
                        value
                    }
                    Self::Allpass {
                        coefficients,
                        histories,
                        odd,
                    } => {
                        let [s, t] = &mut histories[c];
                        let value = 0.5 * (cascade(s, &coefficients[0], f64::from(x)) + odd[c]);
                        odd[c] = cascade(t, &coefficients[1], f64::from(y));
                        value
                    }
                };
                *out = value as f32;
            }
        }
    }
}

fn cascade(state: &mut [f64], coefficients: &[f64], mut input: f64) -> f64 {
    for (s, &a) in state.iter_mut().zip(coefficients) {
        input = allpass(s, input, a);
    }
    input
}

/// One to four independently configured 2× stages, ordered from base to highest rate.
pub struct CustomOversampler {
    spec: ProcessSpec,
    high: ProcessSpec,
    stages: Vec<(Direction, Direction)>,
    work: Vec<f32>,
    scratch: Vec<f32>,
    delay: f64,
}

impl CustomOversampler {
    pub fn new(spec: ProcessSpec, stages: &[StageSpec]) -> Result<Self, Error> {
        spec.validate()?;
        if !(1..=4).contains(&stages.len()) {
            return Err(Error::InvalidSpec);
        }
        let ratio = 1usize << stages.len();
        let high = ProcessSpec {
            max_block_frames: spec
                .max_block_frames
                .checked_mul(ratio)
                .ok_or(Error::InvalidSpec)?,
            sample_rate: spec.sample_rate * ratio as f64,
            ..spec
        }
        .validate()?;
        let samples = high
            .max_block_frames
            .checked_mul(high.channels)
            .filter(|&count| count <= 2_097_152)
            .ok_or(Error::InvalidSpec)?;
        let mut delay = 0.;
        let stages: Vec<_> = stages
            .iter()
            .enumerate()
            .map(|(i, stage)| {
                let up = HalfBand::design(stage.family, stage.up)?;
                let down = HalfBand::design(stage.family, stage.down)?;
                delay += (up.delay_at_lower_rate() + down.delay_at_lower_rate()) / (1 << i) as f64;
                Ok((
                    Direction::new(up, spec.channels, true),
                    Direction::new(down, spec.channels, false),
                ))
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            spec,
            high,
            stages,
            work: vec![0.; samples],
            scratch: vec![0.; samples],
            delay,
        })
    }

    pub fn high_rate_spec(&self) -> ProcessSpec {
        self.high
    }

    /// Round-trip group delay at DC in base-rate samples. FIR-only is linear phase.
    pub fn dc_delay_samples(&self) -> f64 {
        self.delay
    }

    pub fn reset(&mut self) {
        for (up, down) in &mut self.stages {
            up.reset();
            down.reset();
        }
    }

    /// Processes in place. The closure receives a temporary block at the prepared high rate.
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
        let mut len = block.samples().len();
        if len == 0 {
            return Ok(());
        }
        self.stages[0]
            .0
            .up(block.samples(), &mut self.work[..2 * len], channels);
        len *= 2;
        for (up, _) in &mut self.stages[1..] {
            up.up(&self.work[..len], &mut self.scratch[..2 * len], channels);
            len *= 2;
            std::mem::swap(&mut self.work, &mut self.scratch);
        }
        render(
            &mut AudioBuffer::new(&mut self.work[..len], channels)?,
            self.high,
        );
        if self.work[..len].iter().any(|x| !x.is_finite()) {
            self.reset();
            block.clear();
            return Err(Error::InvalidBuffer);
        }
        for (_, down) in self.stages[1..].iter_mut().rev() {
            down.down(&self.work[..len], &mut self.scratch[..len / 2], channels);
            len /= 2;
            std::mem::swap(&mut self.work, &mut self.scratch);
        }
        self.stages[0]
            .1
            .down(&self.work[..len], block.samples_mut(), channels);
        if block.samples().iter().any(|x| !x.is_finite()) {
            self.reset();
            block.clear();
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specifications_hold_on_independent_grid() {
        for family in [Family::EquirippleFir, Family::PolyphaseIir] {
            for (width, db) in [(0.02, 60.), (0.06, 84.), (0.12, 96.), (0.24, 120.)] {
                let f = HalfBand::design(
                    family,
                    FilterSpec {
                        transition_width: width,
                        attenuation_db: db,
                    },
                )
                .unwrap_or_else(|e| panic!("{family:?} width={width} dB={db}: {e}"));
                for i in 0..=11073 {
                    let f0 = (0.25 - width * 0.5) * i as f64 / 11073.;
                    let ripple = 10f64.powf(-db / 20.);
                    assert!((f.magnitude_at(f0).unwrap() - 1.).abs() <= ripple);
                    assert!(f.magnitude_at(0.5 - f0).unwrap() <= ripple);
                }
                if let Some(taps) = f.fir_taps() {
                    let center = taps.len() / 2;
                    assert_eq!(taps[center], 0.5);
                    for (i, &tap) in taps.iter().enumerate() {
                        assert_eq!(tap, taps[2 * center - i]);
                        if i % 2 == 1 && i != center {
                            assert_eq!(tap, 0.);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mixed_pipeline_is_partition_independent_and_recovers() {
        let spec = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 123,
            channels: 2,
        };
        let filter = FilterSpec {
            transition_width: 0.09,
            attenuation_db: 64.,
        };
        let stages = [
            StageSpec {
                family: Family::EquirippleFir,
                up: filter,
                down: FilterSpec {
                    attenuation_db: 48.,
                    ..filter
                },
            },
            StageSpec {
                family: Family::PolyphaseIir,
                up: filter,
                down: filter,
            },
        ];
        let mut processor = CustomOversampler::new(spec, &stages).unwrap();
        let input: Vec<_> = (0..1600)
            .map(|i| if i % 2 == 0 { 1. } else { 0. })
            .collect();
        let render = |processor: &mut CustomOversampler, frames: usize| {
            let mut output = input.clone();
            for chunk in output.chunks_mut(frames * 2) {
                processor
                    .process_with(&mut AudioBuffer::new(chunk, 2).unwrap(), |b, h| {
                        assert_eq!(h.sample_rate, 192000.);
                        assert_eq!(b.frames() % 4, 0);
                    })
                    .unwrap();
            }
            output
        };
        let a = render(&mut processor, 123);
        processor.reset();
        let b = render(&mut processor, 17);
        assert_eq!(a, b);
        assert!(a[1300..]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|s| (s[0] - 1.).abs() < 0.005 && s[1] == 0.));
        let mut bad = [0.5; 10];
        assert!(processor
            .process_with(&mut AudioBuffer::new(&mut bad, 2).unwrap(), |b, _| b
                .samples_mut()[0] =
                f32::NAN)
            .is_err());
        assert_eq!(bad, [0.; 10]);
        assert_eq!(a, render(&mut processor, 9));
    }

    #[test]
    fn roundtrip_delay_matches_impulse_centroid() {
        let spec = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        };
        let up = FilterSpec {
            transition_width: 0.10,
            attenuation_db: 74.,
        };
        let down = FilterSpec {
            transition_width: 0.06,
            attenuation_db: 60.,
        };
        for family in [Family::EquirippleFir, Family::PolyphaseIir] {
            let mut f = CustomOversampler::new(spec, &[StageSpec { family, up, down }]).unwrap();
            let mut h = vec![0.; 1024];
            h[0] = 1.;
            for block in h.chunks_mut(128) {
                f.process_with(&mut AudioBuffer::new(block, 1).unwrap(), |_, _| {})
                    .unwrap();
            }
            let sum = h.iter().map(|&x| f64::from(x)).sum::<f64>();
            let centroid = h
                .iter()
                .enumerate()
                .map(|(i, &x)| i as f64 * f64::from(x))
                .sum::<f64>()
                / sum;
            assert!(
                (centroid - f.dc_delay_samples()).abs() < 0.003,
                "{family:?}: {centroid} vs {}",
                f.dc_delay_samples()
            );
        }
    }
}
