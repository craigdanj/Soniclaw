//! Pitch, stereo and loudness analysis. Loudness is explicitly mono/stereo 48 kHz.
use soniclaw_core::Error;
/// Annex-2 four-phase, 12-tap-per-phase coefficients from ITU-R BS.1770-5.
const TP: [[f32; 4]; 12] = [
    [0.0017089844, -0.029174805, -0.018920898, -0.008300781],
    [0.010986328, 0.029296875, 0.033081055, 0.014892578],
    [-0.01965332, -0.051757813, -0.05822754, -0.026611328],
    [0.033203125, 0.08911133, 0.1015625, 0.047607422],
    [-0.059448242, -0.1665039, -0.20031738, -0.10229492],
    [0.1373291, 0.4650879, 0.77978516, 0.97216797],
    [0.97216797, 0.77978516, 0.4650879, 0.1373291],
    [-0.10229492, -0.20031738, -0.1665039, -0.059448242],
    [0.047607422, 0.1015625, 0.08911133, 0.033203125],
    [-0.026611328, -0.05822754, -0.051757813, -0.01965332],
    [0.014892578, 0.033081055, 0.029296875, 0.010986328],
    [-0.008300781, -0.018920898, -0.029174805, 0.0017089844],
];
pub struct TruePeakDetector {
    history: Vec<[f32; 12]>,
    cursor: usize,
    peak: f32,
}
impl TruePeakDetector {
    pub fn new(channels: usize) -> Result<Self, Error> {
        if channels == 0 || channels > 64 {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            history: vec![[0.0; 12]; channels],
            cursor: 0,
            peak: 0.0,
        })
    }
    pub fn push_frame(&mut self, frame: &[f32]) -> f32 {
        if frame.len() != self.history.len() {
            return 0.0;
        }
        let mut peak = 0.0_f32;
        for (&x, h) in frame.iter().zip(&mut self.history) {
            let x = if x.is_finite() { x } else { 0.0 };
            h[self.cursor] = x;
            peak = peak.max(x.abs());
            for phase in 0..4 {
                let mut y = 0.0;
                for (i, c) in TP.iter().enumerate() {
                    y += c[phase] * h[(self.cursor + 12 - i) % 12];
                }
                peak = peak.max(y.abs());
            }
        }
        self.cursor = (self.cursor + 1) % 12;
        self.peak = self.peak.max(peak);
        peak
    }
    pub fn peak_linear(&self) -> f32 {
        self.peak
    }
    pub fn peak_dbtp(&self) -> f32 {
        20.0 * self.peak.log10()
    }
    pub fn reset(&mut self) {
        self.history.fill([0.0; 12]);
        self.cursor = 0;
        self.peak = 0.0;
    }
}
/// Mid/side uses (L+R)/2 and (L-R)/2. Decode is the exact inverse.
pub fn encode_mid_side(left: f32, right: f32) -> [f32; 2] {
    [(left + right) * 0.5, (left - right) * 0.5]
}
pub fn decode_mid_side(mid: f32, side: f32) -> [f32; 2] {
    [mid + side, mid - side]
}
pub fn stereo_width(left: f32, right: f32, width: f32) -> [f32; 2] {
    let [m, s] = encode_mid_side(left, right);
    decode_mid_side(m, s * width)
}
#[derive(Default)]
pub struct StereoMeter {
    cross: f64,
    left: f64,
    right: f64,
    frames: u64,
}
impl StereoMeter {
    pub fn push(&mut self, left: f32, right: f32) {
        if left.is_finite() && right.is_finite() {
            self.cross += left as f64 * right as f64;
            self.left += (left as f64).powi(2);
            self.right += (right as f64).powi(2);
            self.frames += 1;
        }
    }
    pub fn correlation(&self) -> f32 {
        let den = (self.left * self.right).sqrt();
        if den > 1e-30 {
            (self.cross / den).clamp(-1.0, 1.0) as f32
        } else {
            0.0
        }
    }
    pub fn rms(&self) -> [f32; 2] {
        if self.frames == 0 {
            [0.0; 2]
        } else {
            [
                (self.left / self.frames as f64).sqrt() as f32,
                (self.right / self.frames as f64).sqrt() as f32,
            ]
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
/// YIN difference/CMND estimator for a supplied mono analysis window. Work and
/// storage are bounded by the configured window; invoke at an analysis cadence.
pub struct PitchDetector {
    sr: f64,
    min_hz: f64,
    max_hz: f64,
    difference: Vec<f64>,
    window: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct PitchEstimate {
    pub hz: f64,
    pub confidence: f64,
}
impl PitchDetector {
    pub fn new(sr: f64, window: usize, min_hz: f64, max_hz: f64) -> Result<Self, Error> {
        if !sr.is_finite()
            || !(8000.0..=192000.0).contains(&sr)
            || !(64..=8192).contains(&window)
            || !min_hz.is_finite()
            || !max_hz.is_finite()
            || min_hz < 20.0
            || max_hz <= min_hz
            || max_hz > sr * 0.25
            || sr / min_hz > window as f64 / 2.0
        {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            sr,
            min_hz,
            max_hz,
            difference: vec![0.0; window / 2 + 1],
            window,
        })
    }
    pub fn estimate(&mut self, input: &[f32]) -> Option<PitchEstimate> {
        if input.len() != self.window
            || input.iter().any(|x| !x.is_finite())
            || input.iter().map(|x| x * x).sum::<f32>() / (input.len() as f32) < 1e-10
        {
            return None;
        }
        let max = (self.sr / self.min_hz).ceil() as usize;
        let min = (self.sr / self.max_hz).floor().max(2.0) as usize;
        let count = self.window - max;
        let mut running = 0.0;
        self.difference[0] = 1.0;
        for lag in 1..=max {
            let mut sum = 0.0;
            for i in 0..count {
                let d = input[i] as f64 - input[i + lag] as f64;
                sum += d * d;
            }
            running += sum;
            self.difference[lag] = if running > 1e-30 {
                sum * lag as f64 / running
            } else {
                1.0
            };
        }
        let mut lag = min;
        while lag < max {
            if self.difference[lag] < 0.15 {
                while lag + 1 < max && self.difference[lag + 1] < self.difference[lag] {
                    lag += 1;
                }
                let y = self.difference[lag];
                let a = self.difference[lag - 1];
                let b = self.difference[lag + 1];
                let den = a - 2.0 * y + b;
                let delta = if den.abs() > 1e-15 {
                    (0.5 * (a - b) / den).clamp(-0.5, 0.5)
                } else {
                    0.0
                };
                return Some(PitchEstimate {
                    hz: self.sr / (lag as f64 + delta),
                    confidence: (1.0 - y).clamp(0.0, 1.0),
                });
            }
            lag += 1;
        }
        None
    }
}
#[derive(Clone, Copy)]
struct WeightFilter {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl WeightFilter {
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}
/// BS.1770 K-weighted 400 ms/75%-overlap gates. Bounded exact gate history;
/// once full, integrated history freezes and history_full reports the condition.
/// A standards-inspired implementation, not an EBU/ITU-certified meter.
pub struct LoudnessMeter {
    filters: Vec<[WeightFilter; 2]>,
    ring: Vec<f64>,
    cursor: usize,
    frames: u64,
    sum400: f64,
    sum3s: f64,
    blocks: Vec<f64>,
    used: usize,
    pub history_full: bool,
}
fn lufs(energy: f64) -> f64 {
    if energy > 0.0 {
        -0.691 + 10.0 * energy.log10()
    } else {
        f64::NEG_INFINITY
    }
}
impl LoudnessMeter {
    pub fn new(sample_rate: u32, channels: usize, max_blocks: usize) -> Result<Self, Error> {
        if sample_rate != 48000
            || !(1..=2).contains(&channels)
            || !(1..=360000).contains(&max_blocks)
        {
            return Err(Error::InvalidSpec);
        }
        let shelf = WeightFilter {
            b: [1.53512485958697, -2.69169618940638, 1.19839281085285],
            a: [-1.69065929318241, 0.73248077421585],
            z: [0.0; 2],
        };
        let high = WeightFilter {
            b: [1.0, -2.0, 1.0],
            a: [-1.99004745483398, 0.99007225036621],
            z: [0.0; 2],
        };
        Ok(Self {
            filters: vec![[shelf, high]; channels],
            ring: vec![0.0; 144000],
            cursor: 0,
            frames: 0,
            sum400: 0.0,
            sum3s: 0.0,
            blocks: vec![0.0; max_blocks],
            used: 0,
            history_full: false,
        })
    }
    pub fn process(&mut self, input: &[f32]) -> Result<(), Error> {
        let ch = self.filters.len();
        if !input.len().is_multiple_of(ch) || input.iter().any(|x| !x.is_finite()) {
            return Err(Error::InvalidBuffer);
        }
        for frame in input.chunks_exact(ch) {
            let mut energy = 0.0;
            for (&x, f) in frame.iter().zip(&mut self.filters) {
                let y = f[0].tick(x as f64);
                let y = f[1].tick(y);
                energy += y * y;
            }
            let old400 = self.ring[(self.cursor + 144000 - 19200) % 144000];
            self.sum400 += energy - old400;
            self.sum3s += energy - self.ring[self.cursor];
            self.ring[self.cursor] = energy;
            self.cursor = (self.cursor + 1) % 144000;
            self.frames += 1;
            if self.frames >= 19200 && (self.frames - 19200).is_multiple_of(4800) {
                if self.used < self.blocks.len() {
                    self.blocks[self.used] = self.sum400.max(0.0) / 19200.0;
                    self.used += 1;
                } else {
                    self.history_full = true;
                }
            }
        }
        Ok(())
    }
    pub fn momentary_lufs(&self) -> Option<f64> {
        (self.frames >= 19200).then(|| lufs(self.sum400.max(0.0) / 19200.0))
    }
    pub fn short_term_lufs(&self) -> Option<f64> {
        (self.frames >= 144000).then(|| lufs(self.sum3s.max(0.0) / 144000.0))
    }
    pub fn integrated_lufs(&self) -> Option<f64> {
        let mut count = 0;
        let mut sum = 0.0;
        for &x in &self.blocks[..self.used] {
            if lufs(x) > -70.0 {
                count += 1;
                sum += x;
            }
        }
        if count == 0 {
            return None;
        }
        let gate = (lufs(sum / count as f64) - 10.0).max(-70.0);
        sum = 0.0;
        count = 0;
        for &x in &self.blocks[..self.used] {
            if lufs(x) > gate {
                sum += x;
                count += 1;
            }
        }
        (count > 0).then(|| lufs(sum / count as f64))
    }
    pub fn reset(&mut self) {
        for filters in &mut self.filters {
            for f in filters {
                f.z = [0.0; 2];
            }
        }
        self.ring.fill(0.0);
        self.cursor = 0;
        self.frames = 0;
        self.sum400 = 0.0;
        self.sum3s = 0.0;
        self.used = 0;
        self.history_full = false;
    }
}
pub struct StereoWidth {
    channels: usize,
    width: f32,
}
impl Default for StereoWidth {
    fn default() -> Self {
        Self {
            channels: 0,
            width: 1.0,
        }
    }
}
impl StereoWidth {
    pub fn set(&mut self, width: f32) -> Result<(), Error> {
        if !width.is_finite() || !(0.0..=2.0).contains(&width) {
            return Err(Error::InvalidParameter);
        }
        self.width = width;
        Ok(())
    }
}
impl super::LiveDsp for StereoWidth {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0]);
    }
}
impl soniclaw_core::AudioProcessor for StereoWidth {
    fn prepare(&mut self, s: soniclaw_core::ProcessSpec) -> Result<(), Error> {
        super::spec(s)?;
        self.channels = s.channels;
        Ok(())
    }
    fn process(
        &mut self,
        b: &mut soniclaw_core::AudioBuffer<'_>,
        _: &soniclaw_core::ProcessContext<'_>,
    ) {
        if b.channels() != self.channels {
            b.clear();
            return;
        }
        if self.channels >= 2 {
            for f in b.samples_mut().chunks_exact_mut(self.channels) {
                let x = stereo_width(f[0], f[1], self.width);
                f[0] = x[0];
                f[1] = x[1];
            }
        }
    }
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pitch_and_stereo() {
        let mut p = PitchDetector::new(48000.0, 4096, 50.0, 2000.0).unwrap();
        let d: Vec<_> = (0..4096)
            .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 48000.0).sin() as f32)
            .collect();
        let result = p.estimate(&d).unwrap();
        assert!((result.hz - 440.0).abs() < 0.3);
        assert!(result.confidence > 0.98);
        assert!(p.estimate(&vec![0.0; 4096]).is_none());
        let mut m = StereoMeter::default();
        for &x in &d {
            m.push(x, -x);
        }
        assert!((m.correlation() + 1.0).abs() < 1e-6);
        for pair in [[0.5, -0.2], [-0.4, 0.7]] {
            let ms = encode_mid_side(pair[0], pair[1]);
            let lr = decode_mid_side(ms[0], ms[1]);
            assert!((pair[0] - lr[0]).abs() < 1e-6);
            assert!((pair[1] - lr[1]).abs() < 1e-6);
            let mono = stereo_width(pair[0], pair[1], 0.0);
            assert_eq!(mono[0], mono[1]);
        }
    }
    #[test]
    fn true_peak_detects_intersample_crest() {
        let mut tp = TruePeakDetector::new(1).unwrap();
        let mut sample = 0.0_f32;
        for i in 0..2048 {
            let x = (std::f64::consts::TAU * 0.25 * i as f64 + std::f64::consts::FRAC_PI_4).sin()
                as f32;
            sample = sample.max(x.abs());
            tp.push_frame(&[x]);
        }
        assert!(tp.peak_linear() > sample * 1.3);
        assert!(tp.peak_linear() < 1.1);
        tp.reset();
        assert_eq!(tp.peak_linear(), 0.0);
    }
    #[test]
    fn loudness_reference_gate_and_bounded_history() {
        let mut m = LoudnessMeter::new(48000, 1, 64).unwrap();
        let d: Vec<_> = (0..192000)
            .map(|i| 0.1 * (std::f64::consts::TAU * 997.0 * i as f64 / 48000.0).sin() as f32)
            .collect();
        m.process(&d).unwrap();
        assert!((m.integrated_lufs().unwrap() + 23.01).abs() < 0.05);
        assert!((m.short_term_lufs().unwrap() + 23.01).abs() < 0.05);
        m.process(&vec![0.0; 192000]).unwrap();
        assert!(m.history_full);
        assert!((m.integrated_lufs().unwrap() + 23.01).abs() < 1.0);
        m.reset();
        assert!(m.integrated_lufs().is_none());
    }
}
