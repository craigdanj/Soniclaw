//! Fourth-order Linkwitz–Riley mono kernels, with phase alignment for three ways.
//! Recombination is flat in magnitude, with all-pass phase (not sample-identical dry).
use soniclaw_core::Error;
#[derive(Clone, Copy, Default)]
struct Biquad {
    c: [f64; 5],
    z: [f64; 2],
}
impl Biquad {
    fn set(&mut self, sr: f64, hz: f64, high: bool) {
        let w = std::f64::consts::TAU * hz / sr;
        let cs = w.cos();
        let alpha = w.sin() / 2.0_f64.sqrt();
        let a0 = 1.0 + alpha;
        let b = if high {
            (1.0 + cs) * 0.5
        } else {
            (1.0 - cs) * 0.5
        };
        self.c = [
            b / a0,
            if high { -2.0 * b / a0 } else { 2.0 * b / a0 },
            b / a0,
            -2.0 * cs / a0,
            (1.0 - alpha) / a0,
        ];
    }
    fn tick(&mut self, x: f32) -> f32 {
        let c = self.c;
        let y = c[0] * x as f64 + self.z[0];
        self.z[0] = c[1] * x as f64 - c[3] * y + self.z[1];
        self.z[1] = c[2] * x as f64 - c[4] * y;
        y as f32
    }
    fn reset(&mut self) {
        self.z = [0.0; 2];
    }
}
#[derive(Clone, Copy)]
pub struct LinkwitzRiley4 {
    low: [Biquad; 2],
    high: [Biquad; 2],
    sr: f64,
}
impl LinkwitzRiley4 {
    pub fn new(sample_rate: f64, frequency: f64) -> Result<Self, Error> {
        let mut f = Self {
            low: [Biquad::default(); 2],
            high: [Biquad::default(); 2],
            sr: sample_rate,
        };
        f.set_frequency(frequency)?;
        Ok(f)
    }
    pub fn set_frequency(&mut self, hz: f64) -> Result<(), Error> {
        if !self.sr.is_finite()
            || !(8000.0..=192000.0).contains(&self.sr)
            || !hz.is_finite()
            || !(20.0..=self.sr * 0.45).contains(&hz)
        {
            return Err(Error::InvalidParameter);
        }
        for f in &mut self.low {
            f.set(self.sr, hz, false);
        }
        for f in &mut self.high {
            f.set(self.sr, hz, true);
        }
        Ok(())
    }
    pub fn split(&mut self, x: f32) -> [f32; 2] {
        let low = self.low[0].tick(x);
        let low = self.low[1].tick(low);
        let high = self.high[0].tick(x);
        let high = self.high[1].tick(high);
        [low, high]
    }
    pub fn reset(&mut self) {
        for b in self.low.iter_mut().chain(&mut self.high) {
            b.reset();
        }
    }
}
pub struct ThreeWayCrossover {
    first: LinkwitzRiley4,
    upper: LinkwitzRiley4,
    align: LinkwitzRiley4,
}
impl ThreeWayCrossover {
    pub fn new(sr: f64, low: f64, high: f64) -> Result<Self, Error> {
        if low >= high {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            first: LinkwitzRiley4::new(sr, low)?,
            upper: LinkwitzRiley4::new(sr, high)?,
            align: LinkwitzRiley4::new(sr, high)?,
        })
    }
    pub fn set_frequencies(&mut self, low: f64, high: f64) -> Result<(), Error> {
        if !low.is_finite()
            || !high.is_finite()
            || low >= high
            || low < 20.0
            || high > self.first.sr * 0.45
        {
            return Err(Error::InvalidParameter);
        }
        self.first.set_frequency(low)?;
        self.upper.set_frequency(high)?;
        self.align.set_frequency(high)?;
        Ok(())
    }
    pub fn split(&mut self, x: f32) -> [f32; 3] {
        let [low, rest] = self.first.split(x);
        let [mid, high] = self.upper.split(rest);
        let a = self.align.split(low);
        [a[0] + a[1], mid, high]
    }
    pub fn reset(&mut self) {
        self.first.reset();
        self.upper.reset();
        self.align.reset();
    }
}
/// One band of the phase-aligned three-way splitter, as an in-place effect.
pub struct CrossoverBand {
    filters: Vec<ThreeWayCrossover>,
    sr: f64,
    low: f64,
    high: f64,
    band: usize,
}
impl CrossoverBand {
    pub fn new(band: usize) -> Result<Self, Error> {
        if band > 2 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            filters: vec![],
            sr: 48000.0,
            low: 300.0,
            high: 3000.0,
            band,
        })
    }
    pub fn set(&mut self, low: f32, high: f32) -> Result<(), Error> {
        if !low.is_finite()
            || !high.is_finite()
            || !(40.0..=1000.0).contains(&low)
            || !(1200.0..=16000.0).contains(&high)
        {
            return Err(Error::InvalidParameter);
        }
        self.low = low as f64;
        self.high = (high as f64).min(self.sr * 0.45);
        for f in &mut self.filters {
            f.set_frequencies(self.low, self.high)?;
        }
        Ok(())
    }
}
impl super::LiveDsp for CrossoverBand {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl soniclaw_core::AudioProcessor for CrossoverBand {
    fn prepare(&mut self, s: soniclaw_core::ProcessSpec) -> Result<(), Error> {
        super::spec(s)?;
        self.sr = s.sample_rate;
        self.filters = (0..s.channels)
            .map(|_| {
                ThreeWayCrossover::new(s.sample_rate, self.low, self.high.min(s.sample_rate * 0.45))
            })
            .collect::<Result<_, _>>()?;
        Ok(())
    }
    fn process(
        &mut self,
        b: &mut soniclaw_core::AudioBuffer<'_>,
        _: &soniclaw_core::ProcessContext<'_>,
    ) {
        let ch = b.channels();
        if ch != self.filters.len() {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            for (x, f) in frame.iter_mut().zip(&mut self.filters) {
                *x = f.split(*x)[self.band];
            }
        }
    }
    fn reset(&mut self) {
        for f in &mut self.filters {
            f.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_and_three_way_recombination_flat() {
        for hz in [40.0, 200.0, 1000.0, 5000.0, 12000.0] {
            let mut two = LinkwitzRiley4::new(48000.0, 1000.0).unwrap();
            let mut three = ThreeWayCrossover::new(48000.0, 300.0, 3000.0).unwrap();
            let mut a = 0.0;
            let mut b = 0.0;
            let mut n = 0;
            for i in 0..48000 {
                let x = (std::f64::consts::TAU * hz * i as f64 / 48000.0).sin() as f32;
                let l = two.split(x);
                let t = three.split(x);
                if i >= 24000 {
                    a += (l[0] + l[1]).powi(2);
                    b += t.iter().sum::<f32>().powi(2);
                    n += 1;
                }
            }
            assert!((a / n as f32 - 0.5).abs() < 0.005, "{hz}: {a}");
            assert!((b / n as f32 - 0.5).abs() < 0.005, "{hz}: {b}");
        }
    }
    #[test]
    fn crossover_branches_are_minus_six_db_at_cutoff() {
        let mut f = LinkwitzRiley4::new(48000.0, 1000.0).unwrap();
        let mut e = [0.0; 2];
        for i in 0..48000 {
            let x = (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin();
            let y = f.split(x);
            if i >= 24000 {
                for j in 0..2 {
                    e[j] += y[j] * y[j];
                }
            }
        }
        for e in e {
            assert!(((e / 24000.0).sqrt() - 0.5 / 2.0_f32.sqrt()).abs() < 0.002);
        }
    }
}
