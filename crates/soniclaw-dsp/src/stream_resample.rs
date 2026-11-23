//! Band-limited, variable-rate streaming sample conversion.
use soniclaw_core::Error;
use std::sync::Arc;
const RADIUS: usize = 40;
const TAPS: usize = RADIUS * 2;
const PHASES: usize = 64;
const BANDS: usize = 64;

pub struct SincTable(Vec<f32>);
impl Default for SincTable {
    fn default() -> Self {
        Self::new()
    }
}
impl SincTable {
    pub fn new() -> Self {
        let mut table = vec![0.; (PHASES + 1) * (BANDS + 1) * TAPS];
        for band in 0..=BANDS {
            let denominator = 1. + 3. * band as f64 / BANDS as f64;
            let cutoff = 0.90 / denominator;
            for phase in 0..=PHASES {
                let row = &mut table[(band * (PHASES + 1) + phase) * TAPS..][..TAPS];
                for (i, k) in row.iter_mut().enumerate() {
                    let x = i as f64 - RADIUS as f64 + 1. - phase as f64 / PHASES as f64;
                    let w = 0.42
                        + 0.5 * (std::f64::consts::PI * x / RADIUS as f64).cos()
                        + 0.08 * (std::f64::consts::TAU * x / RADIUS as f64).cos();
                    let y = std::f64::consts::PI * x;
                    *k = (w * if x.abs() < 1e-12 {
                        cutoff
                    } else {
                        (y * cutoff).sin() / y
                    }) as f32;
                }
                let sum: f32 = row.iter().sum();
                for x in row {
                    *x /= sum;
                }
            }
        }
        Self(table)
    }
    fn row(&self, b: usize, p: usize) -> &[f32] {
        &self.0[(b * (PHASES + 1) + p) * TAPS..][..TAPS]
    }
}

pub struct StreamResampler {
    table: Arc<SincTable>,
    ring: Vec<f32>,
    channels: usize,
    capacity: usize,
    written: u64,
    integer: u64,
    fraction: f64,
    rate: f64,
    target: f64,
    slew: f64,
    finished: bool,
}
impl StreamResampler {
    pub fn new(
        table: Arc<SincTable>,
        channels: usize,
        capacity: usize,
        increment: f64,
        slew: f64,
    ) -> Result<Self, Error> {
        if !(1..=64).contains(&channels)
            || !(128..=65536).contains(&capacity)
            || channels * capacity > 2_097_152
            || !(0.25..=4.).contains(&increment)
            || !(0.000001..=0.1).contains(&slew)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            table,
            ring: vec![0.; channels * capacity],
            channels,
            capacity,
            written: 0,
            integer: 0,
            fraction: 0.,
            rate: increment,
            target: increment,
            slew,
            finished: false,
        })
    }
    pub fn set_increment(&mut self, source_per_output: f64) -> Result<(), Error> {
        if !(0.25..=4.).contains(&source_per_output) {
            return Err(Error::InvalidParameter);
        }
        self.target = source_per_output;
        Ok(())
    }
    pub fn lookahead_source_frames(&self) -> usize {
        RADIUS
    }
    pub fn finish(&mut self) {
        self.finished = true;
    }
    pub fn is_drained(&self) -> bool {
        self.finished && self.integer >= self.written
    }
    pub fn reset(&mut self) {
        self.written = 0;
        self.integer = 0;
        self.fraction = 0.;
        self.rate = self.target;
        self.finished = false;
    }
    pub fn push(&mut self, pcm: &[f32]) -> Result<usize, Error> {
        if self.finished
            || !pcm.len().is_multiple_of(self.channels)
            || pcm.iter().any(|x| !x.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        let retained = (self.written - self.integer.saturating_sub(RADIUS as u64))
            .min(self.capacity as u64) as usize;
        let frames = (self.capacity - retained).min(pcm.len() / self.channels);
        if self.written > u64::MAX - 65536 {
            return Err(Error::InvalidBuffer);
        }
        for frame in pcm[..frames * self.channels].chunks_exact(self.channels) {
            let offset = (self.written % self.capacity as u64) as usize * self.channels;
            self.ring[offset..offset + self.channels].copy_from_slice(frame);
            self.written += 1;
        }
        Ok(frames)
    }
    pub fn pull(&mut self, pcm: &mut [f32]) -> Result<usize, Error> {
        if !pcm.len().is_multiple_of(self.channels) {
            return Err(Error::InvalidBuffer);
        }
        let mut produced = 0;
        for frame in pcm.chunks_exact_mut(self.channels) {
            if self.integer >= self.written
                || (!self.finished && self.integer + RADIUS as u64 >= self.written)
            {
                break;
            }
            self.rate += (self.target - self.rate).clamp(-self.slew, self.slew);
            let band = ((self.rate.max(1.) - 1.) * BANDS as f64 / 3.).clamp(0., BANDS as f64);
            let phase = (self.fraction * PHASES as f64).clamp(0., PHASES as f64);
            let b = (band as usize).min(BANDS - 1);
            let p = (phase as usize).min(PHASES - 1);
            let br = (band - b as f64) as f32;
            let pr = (phase - p as f64) as f32;
            let a = self.table.row(b, p);
            let c = self.table.row(b + 1, p);
            let d = self.table.row(b, p + 1);
            let e = self.table.row(b + 1, p + 1);
            let mut sum = [0.; 64];
            for i in 0..TAPS {
                let Some(pos) = self
                    .integer
                    .checked_add_signed(i as i64 - RADIUS as i64 + 1)
                else {
                    continue;
                };
                if pos >= self.written {
                    continue;
                }
                let k1 = a[i] + (d[i] - a[i]) * pr;
                let k2 = c[i] + (e[i] - c[i]) * pr;
                let coefficient = f64::from(k1 + (k2 - k1) * br);
                let index = (pos % self.capacity as u64) as usize * self.channels;
                for (acc, &x) in sum.iter_mut().zip(&self.ring[index..index + self.channels]) {
                    *acc += coefficient * f64::from(x);
                }
            }
            for (y, x) in frame.iter_mut().zip(sum) {
                *y = x as f32;
            }
            self.fraction += self.rate;
            let skip = self.fraction.floor() as u64;
            self.fraction -= skip as f64;
            self.integer += skip;
            produced += 1;
        }
        Ok(produced)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partition_equivalence_and_exact_length() {
        let table = Arc::new(SincTable::new());
        let render = |step: usize| {
            let mut p = StreamResampler::new(table.clone(), 1, 256, 1.25, 0.001).unwrap();
            let mut result = Vec::new();
            let mut b = [0.; 256];
            for chunk in [1.; 1000].chunks(step) {
                assert_eq!(p.push(chunk).unwrap(), chunk.len());
                let n = p.pull(&mut b).unwrap();
                result.extend_from_slice(&b[..n]);
            }
            p.finish();
            let n = p.pull(&mut b).unwrap();
            result.extend_from_slice(&b[..n]);
            assert!(p.is_drained());
            result
        };
        let a = render(31);
        assert_eq!(a, render(101));
        assert_eq!(a.len(), 800);
        assert!(a[48..750].iter().all(|x| (*x - 1.).abs() < 1e-5));
    }
}
