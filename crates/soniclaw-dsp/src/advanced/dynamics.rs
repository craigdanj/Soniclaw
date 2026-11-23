use super::analysis::TruePeakDetector;
use super::*;
pub struct RmsCompressor {
    sr: f32,
    channels: usize,
    threshold: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    window: f32,
    power: f32,
    reduction: f32,
}
impl Default for RmsCompressor {
    fn default() -> Self {
        Self {
            sr: 48000.0,
            channels: 0,
            threshold: -18.0,
            ratio: 4.0,
            attack: 10.0,
            release: 100.0,
            makeup: 0.0,
            window: 10.0,
            power: 0.0,
            reduction: 0.0,
        }
    }
}
impl RmsCompressor {
    pub fn gain_reduction_db(&self) -> f32 {
        self.reduction
    }
    pub fn set(
        &mut self,
        threshold: f32,
        ratio: f32,
        attack: f32,
        release: f32,
        makeup: f32,
        window: f32,
    ) -> Result<(), Error> {
        let values = [threshold, ratio, attack, release, makeup, window];
        let ranges = [
            -100.0..=0.0,
            1.0..=100.0,
            0.0..=1000.0,
            1.0..=10000.0,
            -36.0..=36.0,
            1.0..=100.0,
        ];
        if values
            .iter()
            .zip(ranges)
            .any(|(v, r)| !v.is_finite() || !r.contains(v))
        {
            return Err(Error::InvalidParameter);
        }
        self.threshold = threshold;
        self.ratio = ratio;
        self.attack = attack;
        self.release = release;
        self.makeup = makeup;
        self.window = window;
        Ok(())
    }
}
impl LiveDsp for RmsCompressor {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1], v[2], v[3], v[4], v[5]);
    }
}
impl AudioProcessor for RmsCompressor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels {
            b.clear();
            return;
        }
        let rms = pole(self.window, self.sr);
        let attack = pole(self.attack, self.sr);
        let release = pole(self.release, self.sr);
        for f in b.samples_mut().chunks_exact_mut(self.channels) {
            let p = f.iter().fold(0.0_f32, |a, x| a.max(x * x));
            self.power = clean(rms * self.power + (1.0 - rms) * p);
            let level = 10.0 * self.power.max(1e-20).log10();
            let target = (level - self.threshold).max(0.0) * (1.0 - 1.0 / self.ratio);
            let c = if target > self.reduction {
                attack
            } else {
                release
            };
            self.reduction = clean(c * self.reduction + (1.0 - c) * target);
            let gain = crate::db_to_gain(self.makeup - self.reduction);
            for x in f {
                *x *= gain;
            }
        }
    }
    fn reset(&mut self) {
        self.power = 0.0;
        self.reduction = 0.0;
    }
}
pub struct TransientShaper {
    sr: f32,
    channels: usize,
    attack_db: f32,
    sustain_db: f32,
    fast: f32,
    slow: f32,
}
impl Default for TransientShaper {
    fn default() -> Self {
        Self {
            sr: 48000.0,
            channels: 0,
            attack_db: 0.0,
            sustain_db: 0.0,
            fast: 0.0,
            slow: 0.0,
        }
    }
}
impl TransientShaper {
    pub fn set(&mut self, attack_db: f32, sustain_db: f32) -> Result<(), Error> {
        if [attack_db, sustain_db]
            .iter()
            .any(|v| !v.is_finite() || !(-24.0..=24.0).contains(v))
        {
            return Err(Error::InvalidParameter);
        }
        self.attack_db = attack_db;
        self.sustain_db = sustain_db;
        Ok(())
    }
}
impl LiveDsp for TransientShaper {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for TransientShaper {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels {
            b.clear();
            return;
        }
        let fast = pole(1.0, self.sr);
        let slow = pole(40.0, self.sr);
        for f in b.samples_mut().chunks_exact_mut(self.channels) {
            let level = f.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
            self.fast = clean(fast * self.fast + (1.0 - fast) * level);
            self.slow = clean(slow * self.slow + (1.0 - slow) * level);
            let delta = ((self.fast - self.slow) / (self.slow + 1e-5)).clamp(-1.0, 1.0);
            let gain = crate::db_to_gain(
                self.attack_db * delta.max(0.0) + self.sustain_db * (-delta).max(0.0),
            );
            for x in f {
                *x *= gain;
            }
        }
    }
    fn reset(&mut self) {
        self.fast = 0.0;
        self.slow = 0.0;
    }
}
/// Linked-channel lookahead limiter driven by a 4x FIR peak estimate. A 0.3 dB
/// guard margin is applied; this is not a universal true-peak compliance proof.
pub struct LookaheadLimiter {
    sr: f32,
    channels: usize,
    lookahead_ms: f32,
    ceiling_db: f32,
    release_ms: f32,
    detector: TruePeakDetector,
    ring: Vec<f32>,
    cursor: usize,
    delay: usize,
    gain: f32,
    queue: Vec<(u64, f32)>,
    head: usize,
    len: usize,
    time: u64,
}
impl Default for LookaheadLimiter {
    fn default() -> Self {
        Self {
            sr: 48000.0,
            channels: 0,
            lookahead_ms: 5.0,
            ceiling_db: -1.0,
            release_ms: 100.0,
            detector: TruePeakDetector::new(1).expect("one channel"),
            ring: vec![],
            cursor: 0,
            delay: 0,
            gain: 1.0,
            queue: vec![],
            head: 0,
            len: 0,
            time: 0,
        }
    }
}
impl LookaheadLimiter {
    pub fn set_lookahead_ms(&mut self, ms: f32) -> Result<(), Error> {
        if !ms.is_finite() || !(0.0..=20.0).contains(&ms) {
            return Err(Error::InvalidParameter);
        }
        self.lookahead_ms = ms;
        Ok(())
    }
    pub fn set(&mut self, ceiling_db: f32, release_ms: f32) -> Result<(), Error> {
        if !ceiling_db.is_finite()
            || !(-24.0..=0.0).contains(&ceiling_db)
            || !release_ms.is_finite()
            || !(1.0..=2000.0).contains(&release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        self.ceiling_db = ceiling_db;
        self.release_ms = release_ms;
        Ok(())
    }
    pub fn gain_reduction_db(&self) -> f32 {
        -20.0 * self.gain.max(1e-20).log10()
    }
}
impl LiveDsp for LookaheadLimiter {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for LookaheadLimiter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.channels = s.channels;
        self.delay = (self.lookahead_ms as f64 * s.sample_rate / 1000.0).ceil() as usize + 12;
        self.ring = vec![0.0; self.delay * s.channels];
        self.queue = vec![(0, 0.0); self.delay + 14];
        self.detector = TruePeakDetector::new(s.channels)?;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if b.channels() != self.channels || self.delay == 0 {
            b.clear();
            return;
        }
        let ceiling = crate::db_to_gain(self.ceiling_db - 0.3);
        let release = pole(self.release_ms, self.sr);
        let cap = self.queue.len();
        for f in b.samples_mut().chunks_exact_mut(self.channels) {
            let peak = self.detector.push_frame(f);
            while self.len > 0 && self.queue[(self.head + self.len - 1) % cap].1 <= peak {
                self.len -= 1;
            }
            self.queue[(self.head + self.len) % cap] = (self.time, peak);
            self.len += 1;
            while self.len > 0
                && self.time.saturating_sub(self.queue[self.head].0) > (self.delay + 12) as u64
            {
                self.head = (self.head + 1) % cap;
                self.len -= 1;
            }
            let p = self.queue[self.head].1;
            let target = (ceiling / p.max(1e-20)).min(1.0);
            self.gain = if target < self.gain {
                target
            } else {
                release * self.gain + (1.0 - release) * target
            };
            for (c, x) in f.iter_mut().enumerate() {
                let idx = self.cursor * self.channels + c;
                let old = self.ring[idx];
                self.ring[idx] = *x;
                *x = old * self.gain;
            }
            self.cursor = (self.cursor + 1) % self.delay;
            self.time = self.time.saturating_add(1);
        }
    }
    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.detector.reset();
        self.cursor = 0;
        self.gain = 1.0;
        self.head = 0;
        self.len = 0;
        self.time = 0;
    }
    fn latency_frames(&self) -> usize {
        self.delay
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
    fn rms_ratio_and_transient_bypass() {
        let mut c = RmsCompressor::default();
        c.set(-20.0, 4.0, 0.0, 100.0, 0.0, 10.0).unwrap();
        c.prepare(sp()).unwrap();
        let mut data = vec![1.0; 48000];
        c.process(&mut AudioBuffer::new(&mut data, 1).unwrap(), &context());
        assert!((data[47999] - crate::db_to_gain(-15.0)).abs() < 0.001);
        let mut t = TransientShaper::default();
        t.prepare(sp()).unwrap();
        let old = data.clone();
        t.process(&mut AudioBuffer::new(&mut data, 1).unwrap(), &context());
        assert_eq!(data, old);
        t.set(12.0, 0.0).unwrap();
        t.reset();
        let mut hit = vec![0.5; 256];
        t.process(&mut AudioBuffer::new(&mut hit, 1).unwrap(), &context());
        assert!(hit[20] > 0.6);
    }
    #[test]
    fn lookahead_latency_and_intersample_peak() {
        let mut l = LookaheadLimiter::default();
        l.prepare(sp()).unwrap();
        let delay = l.latency_frames();
        let mut data = vec![0.0; 4096];
        data[0] = 0.1;
        l.process(&mut AudioBuffer::new(&mut data, 1).unwrap(), &context());
        assert!(data[..delay].iter().all(|x| *x == 0.0));
        assert_eq!(data[delay], 0.1);
        l.reset();
        let mut wave: Vec<_> = (0..48000)
            .map(|i| {
                1.4 * (std::f64::consts::TAU * 0.25 * i as f64 + std::f64::consts::FRAC_PI_4).sin()
                    as f32
            })
            .collect();
        l.process(&mut AudioBuffer::new(&mut wave, 1).unwrap(), &context());
        let mut meter = TruePeakDetector::new(1).unwrap();
        for x in wave {
            meter.push_frame(&[x]);
        }
        assert!(
            meter.peak_linear() <= crate::db_to_gain(-1.0) + 0.002,
            "{}",
            meter.peak_dbtp()
        );
        assert!(l.gain_reduction_db() > 2.0);
    }
}
