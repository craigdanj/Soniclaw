//! Three-band linked dynamics/saturation and a signal-dependent peaking equalizer.
use crate::{
    advanced::{
        clean,
        crossover::ThreeWayCrossover,
        filters::{FilterMode, StateVariableFilter},
        pole, spec, LiveDsp,
    },
    db_to_gain, EqBand, EqShape, ParametricEq,
};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug)]
pub struct BandSettings {
    pub threshold_db: f32,
    pub ratio: f32,
    pub gain_db: f32,
    pub drive: f32,
    pub mute: bool,
    pub solo: bool,
}
impl Default for BandSettings {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 1.0,
            gain_db: 0.0,
            drive: 0.0,
            mute: false,
            solo: false,
        }
    }
}
pub struct MultibandRack {
    filters: Vec<ThreeWayCrossover>,
    bands: [BandSettings; 3],
    power: [f32; 3],
    reduction: [f32; 3],
    audible: [SmoothedValue; 3],
    sr: f32,
    low: f32,
    high: f32,
    attack: f32,
    release: f32,
}
impl Default for MultibandRack {
    fn default() -> Self {
        Self {
            filters: vec![],
            bands: [BandSettings::default(); 3],
            power: [0.0; 3],
            reduction: [0.0; 3],
            audible: std::array::from_fn(|_| SmoothedValue::new(1.0, 480)),
            sr: 48000.0,
            low: 300.0,
            high: 3000.0,
            attack: 0.0,
            release: 0.0,
        }
    }
}
impl MultibandRack {
    pub fn set_crossovers(&mut self, low: f32, high: f32) -> Result<(), Error> {
        if !low.is_finite()
            || !high.is_finite()
            || !(40.0..=1000.0).contains(&low)
            || !(1200.0..=16000.0).contains(&high)
        {
            return Err(Error::InvalidParameter);
        }
        if self.low != low || self.high != high {
            for f in &mut self.filters {
                f.set_frequencies(low as f64, (high as f64).min(self.sr as f64 * 0.45))?;
            }
        }
        self.low = low;
        self.high = high;
        Ok(())
    }
    pub fn set_band(&mut self, index: usize, settings: BandSettings) -> Result<(), Error> {
        if index >= 3
            || [
                settings.threshold_db,
                settings.ratio,
                settings.gain_db,
                settings.drive,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || !(-60.0..=0.0).contains(&settings.threshold_db)
            || !(1.0..=20.0).contains(&settings.ratio)
            || !(-24.0..=24.0).contains(&settings.gain_db)
            || !(0.0..=12.0).contains(&settings.drive)
        {
            return Err(Error::InvalidParameter);
        }
        self.bands[index] = settings;
        Ok(())
    }
    pub fn gain_reductions_db(&self) -> [f32; 3] {
        self.reduction
    }
}
impl LiveDsp for MultibandRack {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set_crossovers(v[0], v[1]);
        for band in 0..3 {
            let i = 2 + band * 6;
            let _ = self.set_band(
                band,
                BandSettings {
                    threshold_db: v[i],
                    ratio: v[i + 1],
                    gain_db: v[i + 2],
                    drive: v[i + 3],
                    mute: v[i + 4] >= 0.5,
                    solo: v[i + 5] >= 0.5,
                },
            );
        }
    }
}
impl AudioProcessor for MultibandRack {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.filters = (0..s.channels)
            .map(|_| {
                ThreeWayCrossover::new(
                    s.sample_rate,
                    self.low as f64,
                    (self.high as f64).min(s.sample_rate * 0.45),
                )
            })
            .collect::<Result<_, _>>()?;
        self.audible = std::array::from_fn(|_| SmoothedValue::new(1.0, (self.sr * 0.01) as usize));
        self.attack = pole(10.0, self.sr);
        self.release = pole(100.0, self.sr);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if self.filters.len() != ch {
            b.clear();
            return;
        }
        let any_solo = self.bands.iter().any(|b| b.solo);
        for (i, band) in self.bands.iter().enumerate() {
            self.audible[i].set_target(if !band.mute && (!any_solo || band.solo) {
                1.0
            } else {
                0.0
            });
        }
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            let mut split = [[0.0; 64]; 3];
            for (c, (&x, filter)) in frame.iter().zip(&mut self.filters).enumerate() {
                let y = filter.split(x);
                for band in 0..3 {
                    split[band][c] = y[band];
                }
            }
            frame.fill(0.0);
            for (band, signal) in split.iter_mut().enumerate() {
                let settings = self.bands[band];
                let peak = signal[..ch].iter().fold(0.0_f32, |a, x| a.max(x * x));
                self.power[band] =
                    clean(self.attack * self.power[band] + (1.0 - self.attack) * peak);
                let over = 10.0 * self.power[band].max(1e-20).log10() - settings.threshold_db;
                let target = over.max(0.0) * (1.0 - 1.0 / settings.ratio);
                let c = if target > self.reduction[band] {
                    self.attack
                } else {
                    self.release
                };
                self.reduction[band] = clean(c * self.reduction[band] + (1.0 - c) * target);
                let gain = db_to_gain(settings.gain_db - self.reduction[band])
                    * self.audible[band].next_value();
                for (out, x) in frame.iter_mut().zip(&signal[..ch]) {
                    let y = if settings.drive > 0.0001 {
                        (*x * settings.drive).tanh() / settings.drive.tanh()
                    } else {
                        *x
                    };
                    *out += y * gain;
                }
            }
        }
    }
    fn reset(&mut self) {
        for f in &mut self.filters {
            f.reset();
        }
        self.power = [0.0; 3];
        self.reduction = [0.0; 3];
        let any_solo = self.bands.iter().any(|b| b.solo);
        for (i, b) in self.bands.iter().enumerate() {
            self.audible[i].reset(if !b.mute && (!any_solo || b.solo) {
                1.0
            } else {
                0.0
            });
        }
    }
}
pub struct DynamicEq {
    detector: StateVariableFilter,
    eq: ParametricEq,
    sr: f32,
    channels: usize,
    frequency: f32,
    q: f32,
    threshold: f32,
    range: f32,
    gain: f32,
    attack_ms: f32,
    release_ms: f32,
    power: f32,
    reduction: f32,
    cached: (f32, f32),
}
impl Default for DynamicEq {
    fn default() -> Self {
        Self {
            detector: StateVariableFilter::default(),
            eq: ParametricEq::new(vec![EqBand {
                shape: EqShape::Peak,
                frequency: 1000.0,
                q: 1.0,
                gain_db: 0.0,
            }])
            .expect("valid default EQ"),
            sr: 48000.0,
            channels: 0,
            frequency: 1000.0,
            q: 1.0,
            threshold: -24.0,
            range: -12.0,
            gain: 0.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            power: 0.0,
            reduction: 0.0,
            cached: (f32::NAN, f32::NAN),
        }
    }
}
impl DynamicEq {
    pub fn set(&mut self, v: [f32; 7]) -> Result<(), Error> {
        let ranges = [
            20.0..=20000.0,
            0.1..=10.0,
            -60.0..=0.0,
            -24.0..=24.0,
            -12.0..=12.0,
            1.0..=200.0,
            10.0..=2000.0,
        ];
        if v.iter()
            .zip(ranges)
            .any(|(v, r)| !v.is_finite() || !r.contains(v))
        {
            return Err(Error::InvalidParameter);
        }
        self.frequency = v[0];
        self.q = v[1];
        self.threshold = v[2];
        self.range = v[3];
        self.gain = v[4];
        self.attack_ms = v[5];
        self.release_ms = v[6];
        Ok(())
    }
    pub fn dynamic_gain_db(&self) -> f32 {
        self.reduction
    }
}
impl LiveDsp for DynamicEq {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[..7].try_into().expect("seven controls"));
    }
}
impl AudioProcessor for DynamicEq {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.channels = s.channels;
        self.sr = s.sample_rate as f32;
        self.detector.prepare(s)?;
        self.eq.prepare(s)?;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != self.channels {
            b.clear();
            return;
        }
        if self.cached != (self.frequency, self.q) {
            let _ = self
                .detector
                .set(self.frequency, self.q, 0.0, FilterMode::BandPass);
            self.cached = (self.frequency, self.q);
        }
        let attack = pole(self.attack_ms, self.sr);
        let release = pole(self.release_ms, self.sr);
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            let mut side = [0.0; 64];
            side[..self.channels].copy_from_slice(frame);
            self.detector.process(
                &mut AudioBuffer::new(&mut side[..self.channels], self.channels).expect("frame"),
                ctx,
            );
            let p = side[..self.channels]
                .iter()
                .fold(0.0_f32, |a, x| a.max((x / self.q).powi(2)));
            let c = if p > self.power { attack } else { release };
            self.power = clean(c * self.power + (1.0 - c) * p);
            let level = 10.0 * self.power.max(1e-20).log10();
            let target = self.range * ((level - self.threshold) / 12.0).clamp(0.0, 1.0);
            self.reduction = clean(attack * self.reduction + (1.0 - attack) * target);
            self.eq.update_band(
                0,
                EqBand {
                    shape: EqShape::Peak,
                    frequency: self.frequency,
                    q: self.q,
                    gain_db: (self.gain + self.reduction).clamp(-36.0, 36.0),
                },
                self.sr,
            );
            self.eq.process(
                &mut AudioBuffer::new(frame, self.channels).expect("frame"),
                ctx,
            );
        }
    }
    fn reset(&mut self) {
        self.detector.reset();
        self.eq.reset();
        self.power = 0.0;
        self.reduction = 0.0;
        self.cached = (f32::NAN, f32::NAN);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sp() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 48000,
            channels: 1,
        }
    }
    #[test]
    fn neutral_rack_recombines_and_solo_is_band_selective() {
        let mut rack = MultibandRack::default();
        rack.prepare(sp()).unwrap();
        let mut d: Vec<_> = (0..48000)
            .map(|i| (std::f32::consts::TAU * 100.0 * i as f32 / 48000.0).sin() * 0.1)
            .collect();
        rack.process(
            &mut AudioBuffer::new(&mut d, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        let rms = (d[24000..].iter().map(|x| x * x).sum::<f32>() / 24000.0).sqrt();
        assert!((rms - 0.1 / 2.0_f32.sqrt()).abs() < 0.001);
        rack.set_band(
            2,
            BandSettings {
                solo: true,
                ..Default::default()
            },
        )
        .unwrap();
        rack.reset();
        let mut d: Vec<_> = (0..48000)
            .map(|i| (std::f32::consts::TAU * 100.0 * i as f32 / 48000.0).sin() * 0.1)
            .collect();
        rack.process(
            &mut AudioBuffer::new(&mut d, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert!(d[24000..].iter().all(|x| x.abs() < 0.0001));
    }
    #[test]
    fn dynamic_eq_reduces_loud_center_tone() {
        let mut eq = DynamicEq::default();
        eq.prepare(sp()).unwrap();
        let mut d: Vec<_> = (0..48000)
            .map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.5)
            .collect();
        eq.process(
            &mut AudioBuffer::new(&mut d, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert!(eq.dynamic_gain_db() < -10.0);
        let rms = (d[24000..].iter().map(|x| x * x).sum::<f32>() / 24000.0).sqrt();
        assert!(rms < 0.12);
    }
}
