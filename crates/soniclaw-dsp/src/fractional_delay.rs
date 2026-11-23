//! A shared write head with independently readable fractional taps.
use crate::iir_oversampling::AllpassDelay;
use soniclaw_core::Error;

#[derive(Clone, Copy, Debug)]
pub enum Interpolation {
    Linear,
    Cubic,
}

/// Selectable methods including stateful allpass interpolation.
#[derive(Clone, Copy, Debug)]
pub enum DelayMode {
    Integer,
    Linear,
    Cubic,
    Lagrange3,
    Thiran,
}

/// State for one independent Thiran tap, advanced once per input frame.
#[derive(Default)]
pub struct ThiranTap(AllpassDelay);
impl ThiranTap {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct DelayLine {
    data: Vec<f32>,
    channels: usize,
    width: usize,
    cursor: usize,
    max_delay: usize,
    current: f64,
    target: f64,
    slew: f64,
    mode: DelayMode,
    allpass: Vec<ThiranTap>,
}

impl DelayLine {
    pub fn new(
        channels: usize,
        max_delay: usize,
        initial: f64,
        slew: f64,
        mode: Interpolation,
    ) -> Result<Self, Error> {
        let mode = match mode {
            Interpolation::Linear => DelayMode::Linear,
            Interpolation::Cubic => DelayMode::Cubic,
        };
        Self::with_mode(channels, max_delay, initial, slew, mode)
    }

    pub fn with_mode(
        channels: usize,
        max_delay: usize,
        initial: f64,
        slew: f64,
        mode: DelayMode,
    ) -> Result<Self, Error> {
        if !(1..=64).contains(&channels)
            || !(3..=262_144).contains(&max_delay)
            || !initial.is_finite()
            || !(2. ..=max_delay as f64).contains(&initial)
            || !(0.00001..=1.).contains(&slew)
            || (max_delay + 4) * channels > 4_194_304
        {
            return Err(Error::InvalidParameter);
        }
        let width = max_delay + 4;
        Ok(Self {
            data: vec![0.; width * channels],
            channels,
            width,
            cursor: 0,
            max_delay,
            current: initial,
            target: initial,
            slew,
            mode,
            allpass: if matches!(mode, DelayMode::Thiran) {
                (0..channels).map(|_| ThiranTap::default()).collect()
            } else {
                Vec::new()
            },
        })
    }

    pub fn set_delay(&mut self, samples: f64) -> Result<(), Error> {
        if !(2. ..=self.max_delay as f64).contains(&samples) {
            return Err(Error::InvalidParameter);
        }
        self.target = samples;
        Ok(())
    }

    pub fn delay(&self) -> f64 {
        self.current
    }

    /// Read history before the next input frame is pushed. The newest frame is at delay 1.
    pub fn tap(&self, channel: usize, delay: f64) -> Result<f32, Error> {
        if channel >= self.channels || !(2. ..=self.max_delay as f64).contains(&delay) {
            return Err(Error::InvalidParameter);
        }
        let read = |age| {
            history(
                &self.data,
                self.cursor,
                self.width,
                self.channels,
                channel,
                age,
            )
        };
        if matches!(self.mode, DelayMode::Thiran) {
            return Err(Error::InvalidParameter);
        }
        Ok(interpolate(self.mode, delay, read))
    }

    /// Read older frames through a separate allpass. Invoke once per frame and per tap.
    /// Works with any line mode. Reset or replace the state when changing tap identity.
    pub fn tap_thiran(
        &self,
        channel: usize,
        delay: f64,
        state: &mut ThiranTap,
    ) -> Result<f32, Error> {
        if channel >= self.channels || !(2. ..=self.max_delay as f64).contains(&delay) {
            return Err(Error::InvalidParameter);
        }
        let read = |age| {
            history(
                &self.data,
                self.cursor,
                self.width,
                self.channels,
                channel,
                age,
            )
        };
        Ok(thiran(delay, state, read))
    }

    /// Read and advance one interleaved frame. Invalid sizes leave the line intact.
    pub fn tick(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), Error> {
        if input.len() != self.channels || output.len() != self.channels {
            return Err(Error::InvalidBuffer);
        }
        self.current += (self.target - self.current).clamp(-self.slew, self.slew);
        let offset = self.cursor * self.channels;
        for (ch, (x, y)) in input.iter().zip(output).enumerate() {
            let read = |age| history(&self.data, self.cursor, self.width, self.channels, ch, age);
            *y = match self.mode {
                DelayMode::Thiran => thiran(self.current, &mut self.allpass[ch], read),
                mode => interpolate(mode, self.current, read),
            };
            self.data[offset + ch] = if x.is_finite() { *x } else { 0. };
        }
        self.cursor = (self.cursor + 1) % self.width;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.data.fill(0.);
        self.cursor = 0;
        self.current = self.target;
        for state in &mut self.allpass {
            state.reset();
        }
    }
}

fn history(
    data: &[f32],
    cursor: usize,
    width: usize,
    channels: usize,
    channel: usize,
    age: usize,
) -> f32 {
    data[((cursor + width - age) % width) * channels + channel]
}

fn thiran(delay: f64, state: &mut ThiranTap, read: impl Fn(usize) -> f32) -> f32 {
    // Keep the first-order allpass delay near one sample, where the pole has good margin.
    let integer = (delay + 0.5).floor() as usize - 1;
    state
        .0
        .tick(f64::from(read(integer)), delay - integer as f64) as f32
}

fn interpolate(mode: DelayMode, delay: f64, read: impl Fn(usize) -> f32) -> f32 {
    let index = delay.floor() as usize;
    let f = delay.fract() as f32;
    let a = read(index);
    let b = read(index + 1);
    match mode {
        DelayMode::Integer => {
            if f < 0.5 {
                a
            } else {
                b
            }
        }
        DelayMode::Linear => a + f * (b - a),
        DelayMode::Cubic => {
            let m = read(index - 1);
            let p = read(index + 2);
            a + 0.5 * f * (b - m + f * (2. * m - 5. * a + 4. * b - p + f * (3. * (a - b) + p - m)))
        }
        DelayMode::Lagrange3 => {
            let m = read(index - 1);
            let p = read(index + 2);
            -m * f * (f - 1.) * (f - 2.) / 6. + a * (f + 1.) * (f - 1.) * (f - 2.) / 2.
                - b * (f + 1.) * f * (f - 2.) / 2.
                + p * (f + 1.) * f * (f - 1.) / 6.
        }
        DelayMode::Thiran => unreachable!("stateful mode is dispatched separately"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_impulse_and_bounded_slew() {
        for mode in [Interpolation::Linear, Interpolation::Cubic] {
            let mut d = DelayLine::new(2, 20, 4., 0.25, mode).unwrap();
            let mut out = [0.; 2];
            for i in 0..12 {
                d.tick(&[if i == 0 { 1. } else { 0. }, 0.], &mut out)
                    .unwrap();
                assert_eq!(out, [if i == 4 { 1. } else { 0. }, 0.]);
            }
            assert!(d.set_delay(f64::NAN).is_err());
            d.set_delay(7.).unwrap();
            d.tick(&[0.; 2], &mut out).unwrap();
            assert_eq!(d.delay(), 4.25);
        }
    }
}

#[cfg(test)]
mod interpolation_tests {
    use super::*;

    #[test]
    fn lagrange_reproduces_cubic_polynomial_and_integer_rounds() {
        let polynomial = |age: f32| age * age * age - 2. * age * age + 4.;
        let lagrange = interpolate(DelayMode::Lagrange3, 3.25, |age| polynomial(age as f32));
        assert!((lagrange - polynomial(3.25)).abs() < 1e-5);
        assert_eq!(interpolate(DelayMode::Integer, 3.49, |i| i as f32), 3.);
        assert_eq!(interpolate(DelayMode::Integer, 3.50, |i| i as f32), 4.);
    }

    #[test]
    fn thiran_taps_preserve_magnitude_and_independent_state() {
        let delay = 4.25;
        let mut line = DelayLine::with_mode(2, 32, delay, 1., DelayMode::Thiran).unwrap();
        let mut tap = ThiranTap::default();
        let mut out = [0.; 2];
        let mut impulse = [0.; 512];
        let allocations = allocation_counter::measure(|| {
            for (i, value) in impulse.iter_mut().enumerate() {
                *value = line.tap_thiran(0, delay, &mut tap).unwrap();
                line.tick(&[if i == 0 { 1. } else { 0. }, 0.], &mut out)
                    .unwrap();
                assert_eq!(out, [*value, 0.]);
            }
        });
        assert_eq!(allocations.count_total, 0);
        let response = |f| crate::filter_design::fir_at(&impulse.map(f64::from), f).norm();
        for f in [0., 0.05, 0.1, 0.2, 0.45, 0.5] {
            assert!((response(f) - 1.).abs() < 1e-6);
        }
        let delay_measured: f64 = impulse
            .iter()
            .enumerate()
            .map(|(i, &x)| i as f64 * f64::from(x))
            .sum();
        assert!((delay_measured - delay).abs() < 1e-6);
        line.reset();
        tap.reset();
        assert_eq!(line.tap_thiran(0, delay, &mut tap).unwrap(), 0.);
        assert!(line.tap(0, delay).is_err());
    }

    #[test]
    fn all_modes_agree_for_whole_sample_delay() {
        for mode in [
            DelayMode::Integer,
            DelayMode::Linear,
            DelayMode::Cubic,
            DelayMode::Lagrange3,
            DelayMode::Thiran,
        ] {
            let mut line = DelayLine::with_mode(1, 16, 5., 1., mode).unwrap();
            let mut out = [0.];
            for i in 0..48 {
                line.tick(&[if i == 0 { 1. } else { 0. }], &mut out)
                    .unwrap();
                assert_eq!(out[0], if i == 5 { 1. } else { 0. }, "{mode:?}");
            }
        }
    }
}
