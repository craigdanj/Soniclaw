//! Prepared, allocation-free effects. Configuration changes require exclusive
//! access; coefficient/configuration setters are control-thread operations.
use soniclaw_core::*;
use std::f32::consts::{PI, TAU};

fn valid(values: &[f32]) -> bool {
    values.iter().all(|x| x.is_finite())
}
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}
fn pole(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sr).max(1.0)).exp()
}

#[derive(Clone, Copy, Debug)]
pub enum EqShape {
    LowPass,
    HighPass,
    Peak,
    LowShelf,
    HighShelf,
}
#[derive(Clone, Copy, Debug)]
pub struct EqBand {
    pub shape: EqShape,
    pub frequency: f32,
    pub q: f32,
    pub gain_db: f32,
}
impl EqBand {
    fn coefficients(self, sr: f32) -> Result<[f32; 5], Error> {
        if !valid(&[self.frequency, self.q, self.gain_db])
            || self.frequency <= 0.0
            || self.q <= 0.0
            || self.q > 30.0
            || self.gain_db.abs() > 36.0
        {
            return Err(Error::InvalidParameter);
        }
        let w = TAU * self.frequency.min(sr * 0.49) / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * self.q);
        let a = 10.0_f32.powf(self.gain_db / 40.0);
        let beta = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match self.shape {
            EqShape::LowPass => (
                (1.0 - c) / 2.0,
                1.0 - c,
                (1.0 - c) / 2.0,
                1.0 + alpha,
                -2.0 * c,
                1.0 - alpha,
            ),
            EqShape::HighPass => (
                (1.0 + c) / 2.0,
                -1.0 - c,
                (1.0 + c) / 2.0,
                1.0 + alpha,
                -2.0 * c,
                1.0 - alpha,
            ),
            EqShape::Peak => (
                1.0 + alpha * a,
                -2.0 * c,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * c,
                1.0 - alpha / a,
            ),
            EqShape::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * c + beta),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - beta),
                (a + 1.0) + (a - 1.0) * c + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - beta,
            ),
            EqShape::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * c + beta),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - beta),
                (a + 1.0) - (a - 1.0) * c + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - beta,
            ),
        };
        Ok([b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0])
    }
}
/// RBJ parametric EQ. Set bands while stopped, then prepare. No coefficient
/// interpolation is promised by this configuration-only API.
pub struct ParametricEq {
    bands: Vec<EqBand>,
    coefficients: Vec<[f32; 5]>,
    states: Vec<Vec<[f32; 2]>>,
}
impl ParametricEq {
    pub fn new(bands: Vec<EqBand>) -> Result<Self, Error> {
        if bands.len() > 32 {
            return Err(Error::InvalidParameter);
        }
        for b in &bands {
            b.coefficients(48000.0)?;
        }
        Ok(Self {
            bands,
            coefficients: Vec::new(),
            states: Vec::new(),
        })
    }
}
impl AudioProcessor for ParametricEq {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.coefficients = self
            .bands
            .iter()
            .map(|b| b.coefficients(s.sample_rate as f32))
            .collect::<Result<_, _>>()?;
        self.states = vec![vec![[0.0; 2]; s.channels]; self.bands.len()];
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for (c, states) in self.coefficients.iter().zip(&mut self.states) {
            for frame in b.samples_mut().chunks_exact_mut(ch) {
                for (x, z) in frame.iter_mut().zip(states.iter_mut()) {
                    let y = c[0] * *x + z[0];
                    z[0] = flush(c[1] * *x - c[3] * y + z[1]);
                    z[1] = flush(c[2] * *x - c[4] * y);
                    *x = y;
                }
            }
        }
    }
    fn reset(&mut self) {
        for s in &mut self.states {
            s.fill([0.0; 2]);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CompressorSettings {
    pub threshold_db: f32,
    pub ratio: f32,
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
}
impl Default for CompressorSettings {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            knee_db: 6.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 0.0,
        }
    }
}
/// Stereo-linked feed-forward peak compressor with a soft knee.
pub struct Compressor {
    settings: CompressorSettings,
    attack: f32,
    release: f32,
    reduction: f32,
    makeup: f32,
}
impl Compressor {
    pub fn new(s: CompressorSettings) -> Result<Self, Error> {
        if !valid(&[
            s.threshold_db,
            s.ratio,
            s.knee_db,
            s.attack_ms,
            s.release_ms,
            s.makeup_db,
        ]) || !(-100.0..=0.0).contains(&s.threshold_db)
            || !(1.0..=100.0).contains(&s.ratio)
            || !(0.0..=36.0).contains(&s.knee_db)
            || !(0.0..=1000.0).contains(&s.attack_ms)
            || !(1.0..=10000.0).contains(&s.release_ms)
            || s.makeup_db.abs() > 36.0
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            settings: s,
            attack: 0.0,
            release: 0.0,
            reduction: 0.0,
            makeup: crate::db_to_gain(s.makeup_db),
        })
    }
    pub fn gain_reduction_db(&self) -> f32 {
        self.reduction
    }
}
impl AudioProcessor for Compressor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.attack = pole(self.settings.attack_ms, s.sample_rate as f32);
        self.release = pole(self.settings.release_ms, s.sample_rate as f32);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        let s = self.settings;
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            let peak = frame.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
            let over = 20.0 * peak.max(1e-12).log10() - s.threshold_db;
            let slope = 1.0 - 1.0 / s.ratio;
            let target = if s.knee_db > 0.0 && over.abs() < s.knee_db / 2.0 {
                slope * (over + s.knee_db / 2.0).powi(2) / (2.0 * s.knee_db)
            } else {
                slope * over.max(0.0)
            };
            let c = if target > self.reduction {
                self.attack
            } else {
                self.release
            };
            self.reduction = flush(c * self.reduction + (1.0 - c) * target);
            let g = crate::db_to_gain(-self.reduction) * self.makeup;
            for x in frame {
                *x *= g;
            }
        }
    }
    fn reset(&mut self) {
        self.reduction = 0.0;
    }
}
/// Zero-lookahead, sample-peak ceiling limiter. Does not guarantee intersample
/// true peaks. Instant attack, exponential release, linked across channels.
pub struct PeakLimiter {
    ceiling: f32,
    release_ms: f32,
    release: f32,
    gain: f32,
}
impl PeakLimiter {
    pub fn new(ceiling_db: f32, release_ms: f32) -> Result<Self, Error> {
        if !valid(&[ceiling_db, release_ms])
            || !(-60.0..=0.0).contains(&ceiling_db)
            || !(1.0..=10000.0).contains(&release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            ceiling: crate::db_to_gain(ceiling_db),
            release_ms,
            release: 0.0,
            gain: 1.0,
        })
    }
}
impl AudioProcessor for PeakLimiter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.release = pole(self.release_ms, s.sample_rate as f32);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            let p = f.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
            let target = (self.ceiling / p.max(1e-12)).min(1.0);
            self.gain = if target < self.gain {
                target
            } else {
                self.release * self.gain + (1.0 - self.release) * target
            };
            for x in f {
                *x *= self.gain;
            }
        }
    }
    fn reset(&mut self) {
        self.gain = 1.0;
    }
}
pub struct NoiseGate {
    threshold: f32,
    attack_ms: f32,
    release_ms: f32,
    hold_ms: f32,
    attack: f32,
    release: f32,
    hold_frames: usize,
    remaining: usize,
    gain: f32,
}
impl NoiseGate {
    pub fn new(
        threshold_db: f32,
        attack_ms: f32,
        hold_ms: f32,
        release_ms: f32,
    ) -> Result<Self, Error> {
        if !valid(&[threshold_db, attack_ms, hold_ms, release_ms])
            || !(-120.0..=0.0).contains(&threshold_db)
            || !(0.0..=1000.0).contains(&attack_ms)
            || !(0.0..=10000.0).contains(&hold_ms)
            || !(1.0..=10000.0).contains(&release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            threshold: crate::db_to_gain(threshold_db),
            attack_ms,
            release_ms,
            hold_ms,
            attack: 0.0,
            release: 0.0,
            hold_frames: 0,
            remaining: 0,
            gain: 0.0,
        })
    }
}
impl AudioProcessor for NoiseGate {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.attack = pole(self.attack_ms, s.sample_rate as f32);
        self.release = pole(self.release_ms, s.sample_rate as f32);
        self.hold_frames = (self.hold_ms * 0.001 * s.sample_rate as f32) as usize;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            let loud = f.iter().any(|x| x.abs() >= self.threshold);
            if loud {
                self.remaining = self.hold_frames;
            }
            let open = loud || self.remaining > 0;
            self.remaining = self.remaining.saturating_sub(1);
            let target = if open { 1.0 } else { 0.0 };
            let c = if target > self.gain {
                self.attack
            } else {
                self.release
            };
            self.gain = flush(c * self.gain + (1.0 - c) * target);
            for x in f {
                *x *= self.gain;
            }
        }
    }
    fn reset(&mut self) {
        self.gain = 0.0;
        self.remaining = 0;
    }
}

pub struct Chorus {
    rate: f32,
    depth_ms: f32,
    mix: f32,
    sr: f32,
    phase: f32,
    ring: Vec<f32>,
    cursor: usize,
    frames: usize,
}
impl Chorus {
    pub fn new(rate_hz: f32, depth_ms: f32, mix: f32) -> Result<Self, Error> {
        if !valid(&[rate_hz, depth_ms, mix])
            || !(0.01..=10.0).contains(&rate_hz)
            || !(0.0..=12.0).contains(&depth_ms)
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            rate: rate_hz,
            depth_ms,
            mix,
            sr: 48000.0,
            phase: 0.0,
            ring: Vec::new(),
            cursor: 0,
            frames: 0,
        })
    }
}
impl AudioProcessor for Chorus {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.sr = s.sample_rate as f32;
        self.frames = (self.sr * 0.05).ceil() as usize + 2;
        self.ring = vec![0.0; self.frames * s.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (c, x) in f.iter_mut().enumerate() {
                self.ring[self.cursor * ch + c] = *x;
                let delay = (15.0
                    + self.depth_ms * (self.phase + if c % 2 == 0 { 0.0 } else { PI / 2.0 }).sin())
                    * self.sr
                    * 0.001;
                let pos = (self.cursor as f32 - delay).rem_euclid(self.frames as f32);
                let a = (pos as usize) % self.frames;
                let frac = pos.fract();
                let wet = self.ring[a * ch + c] * (1.0 - frac)
                    + self.ring[((a + 1) % self.frames) * ch + c] * frac;
                *x = *x * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % self.frames;
            self.phase = (self.phase + TAU * self.rate / self.sr) % TAU;
        }
    }
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.phase = 0.0;
        self.cursor = 0;
    }
}
struct Comb {
    data: Vec<f32>,
    cursor: usize,
    damping: f32,
    feedback: f32,
}
impl Comb {
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.data[self.cursor];
        self.damping = flush(0.3 * y + 0.7 * self.damping);
        self.data[self.cursor] = flush(x + self.feedback * self.damping);
        self.cursor = (self.cursor + 1) % self.data.len();
        y
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.cursor = 0;
        self.damping = 0.0;
    }
}
struct AllPass {
    data: Vec<f32>,
    cursor: usize,
}
impl AllPass {
    fn tick(&mut self, x: f32) -> f32 {
        let d = self.data[self.cursor];
        let y = d - 0.5 * x;
        self.data[self.cursor] = flush(x + 0.5 * y);
        self.cursor = (self.cursor + 1) % self.data.len();
        y
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.cursor = 0;
    }
}
/// Schroeder reverb with four damped combs and two allpasses per channel.
pub struct Reverb {
    rt60: f32,
    mix: f32,
    combs: Vec<Vec<Comb>>,
    allpasses: Vec<Vec<AllPass>>,
}
impl Reverb {
    pub fn new(rt60_seconds: f32, mix: f32) -> Result<Self, Error> {
        if !valid(&[rt60_seconds, mix])
            || !(0.1..=20.0).contains(&rt60_seconds)
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            rt60: rt60_seconds,
            mix,
            combs: Vec::new(),
            allpasses: Vec::new(),
        })
    }
}
impl AudioProcessor for Reverb {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        let sr = s.sample_rate as f32;
        self.combs = (0..s.channels)
            .map(|c| {
                [0.0297, 0.0371, 0.0411, 0.0437]
                    .into_iter()
                    .map(|sec| {
                        let n = ((sec + c as f32 * 0.00071) * sr).round().max(1.0) as usize;
                        Comb {
                            data: vec![0.0; n],
                            cursor: 0,
                            damping: 0.0,
                            feedback: 10.0_f32.powf(-3.0 * n as f32 / (sr * self.rt60)),
                        }
                    })
                    .collect()
            })
            .collect();
        self.allpasses = (0..s.channels)
            .map(|c| {
                [0.005, 0.0017]
                    .into_iter()
                    .map(|sec| AllPass {
                        data: vec![
                            0.0;
                            ((sec + c as f32 * 0.00013) * sr).round().max(1.0) as usize
                        ],
                        cursor: 0,
                    })
                    .collect()
            })
            .collect();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            for (c, x) in f.iter_mut().enumerate() {
                let mut y = self.combs[c].iter_mut().map(|d| d.tick(*x)).sum::<f32>() * 0.25;
                for a in &mut self.allpasses[c] {
                    y = a.tick(y);
                }
                *x = *x * (1.0 - self.mix) + y * self.mix;
            }
        }
    }
    fn reset(&mut self) {
        for c in &mut self.combs {
            for d in c {
                d.reset();
            }
        }
        for c in &mut self.allpasses {
            for a in c {
                a.reset();
            }
        }
    }
}

// Audio-owner-only updates used by AutomatedEffect. Storage and topology stay fixed.
impl ParametricEq {
    pub(crate) fn update_band(&mut self, index: usize, band: EqBand, sr: f32) {
        if let Ok(c) = band.coefficients(sr) {
            self.bands[index] = band;
            self.coefficients[index] = c;
        }
    }
}
impl Compressor {
    pub(crate) fn update_live(&mut self, s: CompressorSettings, sr: f32) {
        self.settings = s;
        self.attack = pole(s.attack_ms, sr);
        self.release = pole(s.release_ms, sr);
        self.makeup = crate::db_to_gain(s.makeup_db);
    }
}
impl PeakLimiter {
    pub(crate) fn update_live(&mut self, ceiling: f32, release: f32, sr: f32) {
        self.ceiling = crate::db_to_gain(ceiling);
        self.release = pole(release, sr);
    }
}
impl NoiseGate {
    pub(crate) fn update_live(
        &mut self,
        threshold: f32,
        attack: f32,
        hold: f32,
        release: f32,
        sr: f32,
    ) {
        self.threshold = crate::db_to_gain(threshold);
        self.attack = pole(attack, sr);
        self.release = pole(release, sr);
        self.hold_frames = (hold * 0.001 * sr) as usize;
    }
}
impl Chorus {
    pub(crate) fn update_live(&mut self, rate: f32, depth: f32, mix: f32) {
        self.rate = rate;
        self.depth_ms = depth;
        self.mix = mix;
    }
}
impl Reverb {
    pub(crate) fn update_live(&mut self, rt60: f32, mix: f32, sr: f32) {
        self.mix = mix;
        if self.rt60 != rt60 {
            self.rt60 = rt60;
            for channel in &mut self.combs {
                for comb in channel {
                    comb.feedback = 10.0_f32.powf(-3.0 * comb.data.len() as f32 / (sr * rt60));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 48000,
            channels: 2,
        }
    }
    fn run(p: &mut impl AudioProcessor, data: &mut [f32]) {
        p.process(
            &mut AudioBuffer::new(data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    #[test]
    fn eq_peak_boost_at_center() {
        let mut eq = ParametricEq::new(vec![EqBand {
            shape: EqShape::Peak,
            frequency: 1000.0,
            q: 1.0,
            gain_db: 6.0,
        }])
        .unwrap();
        eq.prepare(spec()).unwrap();
        let mut d: Vec<_> = (0..48000)
            .flat_map(|i| {
                let v = (TAU * 1000.0 * i as f32 / 48000.0).sin();
                [v, v]
            })
            .collect();
        run(&mut eq, &mut d);
        let rms = (d[48000..].iter().map(|x| x * x).sum::<f32>() / 48000.0).sqrt();
        assert!((rms - 10.0_f32.powf(6.0 / 20.0) / 2.0_f32.sqrt()).abs() < 0.01);
    }
    #[test]
    fn dynamics_ceiling_and_ratio() {
        let mut c = Compressor::new(CompressorSettings {
            threshold_db: -20.0,
            ratio: 4.0,
            knee_db: 0.0,
            ..Default::default()
        })
        .unwrap();
        c.prepare(spec()).unwrap();
        let mut d = vec![1.0; 96000];
        run(&mut c, &mut d);
        assert!((d[95999] - crate::db_to_gain(-15.0)).abs() < 0.001);
        let mut l = PeakLimiter::new(-1.0, 100.0).unwrap();
        l.prepare(spec()).unwrap();
        let mut d = vec![4.0, -3.0, 0.1, 0.2];
        run(&mut l, &mut d);
        assert!(d.iter().all(|x| x.abs() <= crate::db_to_gain(-1.0) + 1e-6));
    }
    #[test]
    fn gate_rejects_quiet_and_reset_clears_tails() {
        let mut g = NoiseGate::new(-30.0, 1.0, 10.0, 50.0).unwrap();
        g.prepare(spec()).unwrap();
        let mut d = vec![0.001; 1000];
        run(&mut g, &mut d);
        assert!(d.iter().all(|x| *x == 0.0));
        let mut r = Reverb::new(0.4, 1.0).unwrap();
        r.prepare(spec()).unwrap();
        let mut d = vec![0.0; 96000];
        d[0] = 1.0;
        d[1] = 1.0;
        run(&mut r, &mut d);
        assert!(d[2000..].iter().any(|x| x.abs() > 0.001));
        assert!(d.iter().all(|x| x.is_finite()));
        r.reset();
        d.fill(0.0);
        run(&mut r, &mut d);
        assert!(d.iter().all(|x| *x == 0.0));
    }
    #[test]
    fn chorus_wet_path_and_parameter_rejection() {
        assert!(Chorus::new(1.0, 20.0, 0.5).is_err());
        let mut c = Chorus::new(1.0, 0.0, 1.0).unwrap();
        c.prepare(spec()).unwrap();
        let mut d = vec![0.0; 2000];
        d[0] = 1.0;
        d[1] = 1.0;
        run(&mut c, &mut d);
        assert_eq!(d[1440], 1.0);
        assert_eq!(d[0], 0.0);
    }
}
