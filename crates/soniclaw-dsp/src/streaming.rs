//! Prepared phase-locked pitch shifting and pull-driven stereo-linked WSOLA time stretching.
use crate::advanced::spectral::{Complex32, Stft};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::f32::consts::{PI, TAU};
fn wrap(x: f32) -> f32 {
    (x + PI).rem_euclid(TAU) - PI
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PitchSettings {
    pub schema: u32,
    pub window_frames: usize,
    pub phase_locking: bool,
    pub transient_threshold: Option<f32>,
    pub mix: f32,
}
impl Default for PitchSettings {
    fn default() -> Self {
        Self {
            schema: 1,
            window_frames: 1024,
            phase_locking: true,
            transient_threshold: Some(0.5),
            mix: 1.,
        }
    }
}
impl PitchSettings {
    pub fn validate(self) -> Result<(), Error> {
        if self.schema != 1
            || !self.window_frames.is_power_of_two()
            || !(256..=4096).contains(&self.window_frames)
            || self
                .transient_threshold
                .is_some_and(|v| !v.is_finite() || !(0.05..=4.).contains(&v))
            || !self.mix.is_finite()
            || !(0.0..=1.).contains(&self.mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
/// Optional bounded log-spectral-envelope preservation. This is an approximation, not speech analysis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormantSettings {
    pub schema: u32,
    pub amount: f32,
    pub shift_semitones: f32,
    pub smoothing_hz: f32,
    pub smoothing_ms: f32,
    pub max_gain_db: f32,
}
impl Default for FormantSettings {
    fn default() -> Self {
        Self {
            schema: 1,
            amount: 1.,
            shift_semitones: 0.,
            smoothing_hz: 400.,
            smoothing_ms: 20.,
            max_gain_db: 18.,
        }
    }
}
impl FormantSettings {
    pub fn validate(self) -> Result<(), Error> {
        if self.schema != 1
            || [
                (self.amount, 0., 1.),
                (self.shift_semitones, -12., 12.),
                (self.smoothing_hz, 50., 2000.),
                (self.smoothing_ms, 0., 200.),
                (self.max_gain_db, 0., 24.),
            ]
            .iter()
            .any(|(v, a, b)| !v.is_finite() || v < a || v > b)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
struct PitchChannel {
    envelope: Vec<f32>,
    envelope_prefix: Vec<f64>,
    envelope_ready: bool,
    stft: Stft,
    previous: Vec<f32>,
    source_phase: Vec<f32>,
    magnitude: Vec<f32>,
    phase: Vec<f32>,
    sum: Vec<f32>,
    weighted: Vec<f32>,
    nearest: Vec<usize>,
    peaks: Vec<usize>,
    frames: usize,
    ready: bool,
    resets: u64,
}
impl PitchChannel {
    fn new(size: usize) -> Result<Self, Error> {
        let n = size / 2 + 1;
        Ok(Self {
            envelope: vec![0.; n],
            envelope_prefix: vec![0.; n + 1],
            envelope_ready: false,
            stft: Stft::new(size, size / 4)?,
            previous: vec![0.; n],
            source_phase: vec![0.; n],
            magnitude: vec![0.; n],
            phase: vec![0.; n],
            sum: vec![0.; n],
            weighted: vec![0.; n],
            nearest: vec![0; n],
            peaks: Vec::with_capacity(n),
            frames: 0,
            ready: false,
            resets: 0,
        })
    }
    fn reset(&mut self) {
        self.envelope.fill(0.);
        self.envelope_prefix.fill(0.);
        self.envelope_ready = false;
        self.stft.reset();
        self.previous.fill(0.);
        self.source_phase.fill(0.);
        self.magnitude.fill(0.);
        self.phase.fill(0.);
        self.sum.fill(0.);
        self.weighted.fill(0.);
        self.nearest.fill(0);
        self.peaks.clear();
        self.frames = 0;
        self.ready = false;
        self.resets = 0;
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PitchStats {
    pub transient_channel_resets: u64,
    pub invalid_blocks: u64,
}
pub struct PitchShifter {
    formants: Option<FormantSettings>,
    settings: PitchSettings,
    semitones: f32,
    channels: Vec<PitchChannel>,
    dry: Vec<f32>,
    cursor: usize,
    spec: Option<ProcessSpec>,
    invalid: u64,
}
impl PitchShifter {
    pub fn new(semitones: f32, settings: PitchSettings) -> Result<Self, Error> {
        settings.validate()?;
        if !semitones.is_finite() || !(-12.0..=12.).contains(&semitones) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            formants: None,
            settings,
            semitones,
            channels: vec![],
            dry: vec![],
            cursor: 0,
            spec: None,
            invalid: 0,
        })
    }
    /// Changes targets on the audio owner. All storage is allocated during prepare.
    pub fn set_formants(&mut self, settings: Option<FormantSettings>) -> Result<(), Error> {
        if let Some(s) = settings {
            s.validate()?;
        }
        if self.formants.is_some() != settings.is_some() {
            for ch in &mut self.channels {
                ch.envelope_ready = false;
            }
        }
        self.formants = settings;
        Ok(())
    }
    pub fn formants(&self) -> Option<FormantSettings> {
        self.formants
    }
    pub fn settings(&self) -> PitchSettings {
        self.settings
    }
    /// Applies at the next spectral frame. Schedule on the processing owner; abrupt changes are not smoothed.
    pub fn set_semitones(&mut self, value: f32) -> Result<(), Error> {
        if !value.is_finite() || !(-12.0..=12.).contains(&value) {
            return Err(Error::InvalidParameter);
        }
        self.semitones = value;
        Ok(())
    }
    pub fn stats(&self) -> PitchStats {
        PitchStats {
            transient_channel_resets: self.channels.iter().map(|c| c.resets).sum(),
            invalid_blocks: self.invalid,
        }
    }
}
impl AudioProcessor for PitchShifter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if !(8000.0..=192000.).contains(&s.sample_rate)
            || s.channels > 2
            || s.max_block_frames > 8192
        {
            return Err(Error::InvalidSpec);
        }
        self.channels = (0..s.channels)
            .map(|_| PitchChannel::new(self.settings.window_frames))
            .collect::<Result<_, _>>()?;
        self.dry = vec![0.; self.settings.window_frames * s.channels];
        self.cursor = 0;
        self.spec = Some(s);
        self.invalid = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self
            .spec
            .is_none_or(|s| b.channels() != s.channels || b.frames() > s.max_block_frames)
            || b.samples().iter().any(|v| !v.is_finite() || v.abs() > 1e6)
        {
            b.clear();
            self.invalid = self.invalid.saturating_add(1);
            return;
        }
        let channels = b.channels();
        let size = self.settings.window_frames;
        let half = size / 2;
        let hop = size / 4;
        let ratio = 2f32.powf(self.semitones / 12.);
        let settings = self.settings;
        let formants = self.formants;
        let sample_rate = self.spec.expect("prepared").sample_rate;
        for frame in b.samples_mut().chunks_exact_mut(channels) {
            for (c, sample) in frame.iter_mut().enumerate() {
                let ch = &mut self.channels[c];
                ch.frames = ch.frames.saturating_add(1);
                let wet = ch.stft.tick(*sample, |bins| {
                    ch.sum.fill(0.);
                    ch.weighted.fill(0.);
                    ch.peaks.clear();
                    let (mut flux, mut previous_total) = (0f32, 0f32);
                    for k in 0..=half {
                        let phase = bins[k].arg();
                        let mag = bins[k].norm();
                        flux += (mag - ch.magnitude[k]).max(0.);
                        previous_total += ch.magnitude[k];
                        ch.magnitude[k] = mag;
                        ch.source_phase[k] = phase;
                        let omega = TAU * k as f32 / size as f32;
                        let frequency =
                            omega + wrap(phase - ch.previous[k] - omega * hop as f32) / hop as f32;
                        ch.previous[k] = phase;
                        let dest = k as f32 * ratio;
                        let a = dest.floor() as usize;
                        let fraction = dest.fract();
                        for (j, w) in [(a, 1. - fraction), (a + 1, fraction)] {
                            if j <= half {
                                ch.sum[j] += mag * w;
                                ch.weighted[j] += mag * w * frequency * ratio;
                            }
                        }
                    }
                    if let Some(formants) = formants {
                        ch.envelope_prefix[0] = 0.;
                        for k in 0..=half {
                            ch.envelope_prefix[k + 1] =
                                ch.envelope_prefix[k] + ch.magnitude[k].max(1e-8).ln() as f64;
                        }
                        let radius = ((formants.smoothing_hz as f64 * size as f64 / sample_rate)
                            .ceil() as usize)
                            .clamp(1, 256);
                        let alpha = if formants.smoothing_ms == 0. {
                            1.
                        } else {
                            -(-(hop as f64) / (sample_rate * formants.smoothing_ms as f64 * 0.001))
                                .exp_m1()
                        } as f32;
                        for k in 0..=half {
                            let left = k.saturating_sub(radius);
                            let right = (k + radius).min(half);
                            let value = ((ch.envelope_prefix[right + 1] - ch.envelope_prefix[left])
                                / (right - left + 1) as f64)
                                as f32;
                            if ch.envelope_ready {
                                ch.envelope[k] += alpha * (value - ch.envelope[k]);
                            } else {
                                ch.envelope[k] = value;
                            }
                        }
                        ch.envelope_ready = true;
                    }
                    let eligible = ch.frames >= size;
                    let onset = eligible
                        && ch.ready
                        && settings.transient_threshold.is_some_and(|threshold| {
                            flux > 1e-9
                                && (previous_total < 1e-9 || flux / previous_total > threshold)
                        });
                    let reset = eligible && (!ch.ready || onset);
                    if onset {
                        ch.resets = ch.resets.saturating_add(1)
                    }
                    if eligible {
                        ch.ready = true;
                    }
                    if (ratio - 1.).abs() < 1e-7
                        && formants.is_none_or(|s| s.shift_semitones == 0. || s.amount == 0.)
                    {
                        ch.phase.copy_from_slice(&ch.source_phase);
                        return;
                    }
                    for k in 0..=half {
                        let source = (k as f32 / ratio).round().min(half as f32) as usize;
                        let f = if ch.sum[k] > 1e-12 {
                            ch.weighted[k] / ch.sum[k]
                        } else {
                            TAU * k as f32 / size as f32
                        };
                        ch.phase[k] = if reset {
                            ch.source_phase[source]
                        } else {
                            wrap(ch.phase[k] + f * hop as f32)
                        };
                    }
                    if settings.phase_locking {
                        for k in 1..half {
                            if ch.magnitude[k] > 1e-9
                                && ch.magnitude[k] >= ch.magnitude[k - 1]
                                && ch.magnitude[k] > ch.magnitude[k + 1]
                            {
                                ch.peaks.push(k);
                            }
                        }
                        if !ch.peaks.is_empty() {
                            let mut index = 0;
                            for k in 0..=half {
                                while index + 1 < ch.peaks.len()
                                    && ch.peaks[index + 1].abs_diff(k) < ch.peaks[index].abs_diff(k)
                                {
                                    index += 1;
                                }
                                ch.nearest[k] = ch.peaks[index];
                            }
                        }
                    }
                    for k in 0..=half {
                        let mut phase = ch.phase[k];
                        if settings.phase_locking && !ch.peaks.is_empty() {
                            let source = (k as f32 / ratio).round().min(half as f32) as usize;
                            let peak = ch.nearest[source];
                            let destination = (peak as f32 * ratio).round() as usize;
                            if destination <= half {
                                phase = ch.phase[destination]
                                    + wrap(ch.source_phase[source] - ch.source_phase[peak]);
                            }
                        }
                        let mut magnitude = ch.sum[k];
                        if let Some(settings) = formants {
                            let sample = |position: f32| {
                                let p = position.clamp(0., half as f32);
                                let a = p.floor() as usize;
                                let b = (a + 1).min(half);
                                ch.envelope[a] + (ch.envelope[b] - ch.envelope[a]) * p.fract()
                            };
                            let shifted = 2f32.powf(settings.shift_semitones / 12.);
                            let gain = ((sample(k as f32 / shifted) - sample(k as f32 / ratio))
                                * settings.amount)
                                .clamp(
                                    -settings.max_gain_db * std::f32::consts::LN_10 / 20.,
                                    settings.max_gain_db * std::f32::consts::LN_10 / 20.,
                                );
                            magnitude *= gain.exp();
                        }
                        bins[k] = Complex32::from_polar(magnitude, phase);
                    }
                    bins[0].im = 0.;
                    bins[half].im = 0.;
                    for k in 1..half {
                        bins[size - k] = bins[k].conj();
                    }
                });
                let index = self.cursor * channels + c;
                let dry = self.dry[index];
                self.dry[index] = *sample;
                let out = dry * (1. - settings.mix) + wet * settings.mix;
                *sample = if out.is_finite() { out } else { 0. };
            }
            self.cursor = (self.cursor + 1) % size;
        }
    }
    fn reset(&mut self) {
        for c in &mut self.channels {
            c.reset();
        }
        self.dry.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.settings.window_frames
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StretchSettings {
    pub schema: u32,
    pub channels: usize,
    pub ratio: f64,
    pub window_frames: usize,
    pub search_frames: usize,
    pub max_block_frames: usize,
    pub transient_ratio: Option<f64>,
}
impl Default for StretchSettings {
    fn default() -> Self {
        Self {
            schema: 1,
            channels: 2,
            ratio: 1.25,
            window_frames: 1024,
            search_frames: 128,
            max_block_frames: 1024,
            transient_ratio: Some(4.),
        }
    }
}
impl StretchSettings {
    pub fn validate(self) -> Result<(), Error> {
        if self.schema != 1
            || !(1..=64).contains(&self.channels)
            || !self.ratio.is_finite()
            || !(0.5..=2.).contains(&self.ratio)
            || !self.window_frames.is_power_of_two()
            || !(256..=4096).contains(&self.window_frames)
            || self.search_frames > self.window_frames / 4
            || !(1..=8192).contains(&self.max_block_frames)
            || self
                .transient_ratio
                .is_some_and(|v| !v.is_finite() || !(1.1..=100.).contains(&v))
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
}
/// Variable-I/O WSOLA. Push input, pull bounded output, finish once, then drain. No hidden worker/driver.
pub struct TimeStretcher {
    settings: StretchSettings,
    reference_channel: Option<usize>,
    input: Vec<f32>,
    capacity: usize,
    start: u64,
    total: u64,
    tail: Vec<f32>,
    pending: Vec<f32>,
    pending_read: usize,
    pending_len: usize,
    grain: u64,
    emitted: u64,
    finished: bool,
    goal: u64,
    locks: u64,
}
impl TimeStretcher {
    pub fn new(settings: StretchSettings) -> Result<Self, Error> {
        settings.validate()?;
        let capacity = settings.window_frames * 4 + settings.max_block_frames;
        let half = settings.window_frames / 2;
        Ok(Self {
            settings,
            reference_channel: None,
            input: vec![0.; capacity * settings.channels],
            capacity,
            start: 0,
            total: 0,
            tail: vec![0.; half * settings.channels],
            pending: vec![0.; half * settings.channels],
            pending_read: 0,
            pending_len: 0,
            grain: 0,
            emitted: 0,
            finished: false,
            goal: 0,
            locks: 0,
        })
    }
    /// Select one analysis channel before input; synthesis offsets remain shared across every channel.
    pub fn set_reference_channel(&mut self, channel: Option<usize>) -> Result<(), Error> {
        if self.total != 0
            || self.grain != 0
            || channel.is_some_and(|c| c >= self.settings.channels)
        {
            return Err(Error::InvalidParameter);
        }
        self.reference_channel = channel;
        Ok(())
    }
    pub fn settings(&self) -> StretchSettings {
        self.settings
    }
    pub fn priming_frames(&self) -> usize {
        self.settings.window_frames + self.settings.search_frames
    }
    pub fn input_frames(&self) -> u64 {
        self.total
    }
    pub fn output_frames(&self) -> u64 {
        self.emitted
    }
    pub fn transient_locks(&self) -> u64 {
        self.locks
    }
    pub fn input_space(&self) -> usize {
        self.capacity - (self.total - self.start) as usize
    }
    pub fn is_drained(&self) -> bool {
        self.finished && self.emitted >= self.goal
    }
    /// Returns accepted frames. Resubmit any remainder after pulling output to release input space.
    pub fn push(&mut self, input: &[f32]) -> Result<usize, Error> {
        let ch = self.settings.channels;
        if self.finished {
            return Err(Error::NotPrepared);
        }
        if !input.len().is_multiple_of(ch)
            || input.len() / ch > self.settings.max_block_frames
            || input.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
        {
            return Err(Error::InvalidBuffer);
        }
        let frames = (input.len() / ch).min(self.input_space());
        if self
            .total
            .checked_add(frames as u64)
            .is_none_or(|v| v > 1 << 48)
        {
            return Err(Error::InvalidBuffer);
        }
        for i in 0..frames {
            let index = ((self.total + i as u64) % self.capacity as u64) as usize * ch;
            self.input[index..index + ch].copy_from_slice(&input[i * ch..(i + 1) * ch]);
        }
        self.total += frames as u64;
        Ok(frames)
    }
    pub fn finish(&mut self) {
        self.finished = true;
        self.goal = (self.total as f64 * self.settings.ratio).ceil() as u64;
    }
    /// Output is cleared first; return value is valid produced frames. At most max_grains (0..=64) windows are searched.
    pub fn pull(&mut self, output: &mut [f32], max_grains: usize) -> Result<usize, Error> {
        output.fill(0.);
        let ch = self.settings.channels;
        if !output.len().is_multiple_of(ch)
            || output.len() / ch > self.settings.max_block_frames
            || max_grains > 64
        {
            return Err(Error::InvalidBuffer);
        }
        let mut wanted = output.len() / ch;
        if self.finished {
            wanted = wanted.min(self.goal.saturating_sub(self.emitted).min(wanted as u64) as usize);
        }
        let (mut produced, mut budget) = (0, max_grains);
        while produced < wanted {
            if self.pending_read == self.pending_len {
                if budget == 0 || !self.generate() {
                    break;
                }
                budget -= 1;
            }
            let count = (self.pending_len - self.pending_read).min(wanted - produced);
            output[produced * ch..(produced + count) * ch].copy_from_slice(
                &self.pending[self.pending_read * ch..(self.pending_read + count) * ch],
            );
            self.pending_read += count;
            produced += count;
        }
        self.emitted += produced as u64;
        Ok(produced)
    }
    fn get(&self, frame: u64, channel: usize) -> f32 {
        if frame >= self.total {
            0.
        } else {
            debug_assert!(frame >= self.start);
            self.input[(frame % self.capacity as u64) as usize * self.settings.channels + channel]
        }
    }
    fn energy(&self, start: u64) -> f64 {
        let mut e = 0.;
        for i in 0..self.settings.window_frames / 2 {
            for ch in 0..self.settings.channels {
                if self.reference_channel.is_some_and(|r| r != ch) {
                    continue;
                }
                e += (self.get(start + i as u64, ch) as f64).powi(2);
            }
        }
        e
    }
    fn generate(&mut self) -> bool {
        if self.finished && self.emitted >= self.goal {
            return false;
        }
        let hop = self.settings.window_frames / 2;
        let ch = self.settings.channels;
        let center = (self.grain as f64 * hop as f64 / self.settings.ratio).round() as u64;
        let right = center + self.settings.search_frames as u64;
        if !self.finished
            && ((self.grain == 0 && self.total < self.settings.window_frames as u64)
                || (self.grain > 0 && right + self.settings.window_frames as u64 > self.total))
        {
            return false;
        }
        let tail_energy = self
            .tail
            .iter()
            .enumerate()
            .filter(|(i, _)| self.reference_channel.is_none_or(|r| i % ch == r))
            .map(|(_, v)| (*v as f64).powi(2))
            .sum::<f64>();
        let energy = self.energy(center);
        let transient = self.grain > 0
            && energy > 1e-12
            && self
                .settings
                .transient_ratio
                .is_some_and(|threshold| energy > tail_energy * threshold);
        let mut best = center;
        if self.grain > 0
            && (self.settings.ratio - 1.).abs() > 1e-12
            && !transient
            && tail_energy > 1e-16
            && self.settings.search_frames > 0
        {
            let left = center
                .saturating_sub(self.settings.search_frames as u64)
                .max(self.start);
            let right = right.min(self.total.saturating_sub(1));
            let mut score = f64::NEG_INFINITY;
            for candidate in left..=right {
                let (mut dot, mut a, mut b) = (0., 0., 0.);
                for i in (0..hop).step_by(4) {
                    for channel in 0..ch {
                        if self.reference_channel.is_some_and(|r| r != channel) {
                            continue;
                        }
                        let x = self.tail[i * ch + channel] as f64;
                        let y = self.get(candidate + i as u64, channel) as f64;
                        dot += x * y;
                        a += x * x;
                        b += y * y;
                    }
                }
                let value = dot / (a * b).sqrt().max(1e-30);
                if value > score + 1e-9
                    || ((value - score).abs() <= 1e-9
                        && candidate.abs_diff(center) < best.abs_diff(center))
                {
                    score = value;
                    best = candidate;
                }
            }
        }
        if transient {
            self.locks = self.locks.saturating_add(1);
        }
        for i in 0..hop {
            let fade = (i + 1) as f32 / (hop + 1) as f32;
            for channel in 0..ch {
                let source = self.get(best + i as u64, channel);
                self.pending[i * ch + channel] =
                    if self.grain == 0 || transient || (self.settings.ratio - 1.).abs() < 1e-12 {
                        source
                    } else {
                        self.tail[i * ch + channel] * (1. - fade) + source * fade
                    };
            }
        }
        for i in 0..hop {
            for channel in 0..ch {
                self.tail[i * ch + channel] = self.get(best + (hop + i) as u64, channel);
            }
        }
        self.pending_read = 0;
        self.pending_len = hop;
        self.grain += 1;
        let next = (self.grain as f64 * hop as f64 / self.settings.ratio).floor() as u64;
        self.start = self.start.max(
            next.saturating_sub(self.settings.search_frames as u64)
                .min(self.total),
        );
        true
    }
    pub fn reset(&mut self) {
        self.start = 0;
        self.total = 0;
        self.grain = 0;
        self.emitted = 0;
        self.finished = false;
        self.goal = 0;
        self.locks = 0;
        self.pending_len = 0;
        self.pending_read = 0;
        self.tail.fill(0.);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tone(frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| (std::f64::consts::TAU * 440. * i as f64 / 48000.).sin() as f32 * 0.2)
            .collect()
    }
    fn hz(input: &[f32]) -> f64 {
        let mut first = 0.;
        let mut last = 0.;
        let mut count = 0;
        for (i, w) in input.windows(2).enumerate() {
            if w[0] <= 0. && w[1] > 0. {
                let cross = i as f64 + (-w[0] / (w[1] - w[0])) as f64;
                if count == 0 {
                    first = cross;
                }
                last = cross;
                count += 1;
            }
        }
        assert!(count > 4);
        48000. * (count - 1) as f64 / (last - first)
    }
    fn pitch(
        input: &[f32],
        semitones: f32,
        block: usize,
        settings: PitchSettings,
    ) -> (Vec<f32>, PitchStats) {
        let mut p = PitchShifter::new(semitones, settings).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 1024,
        })
        .unwrap();
        let mut output = input.to_vec();
        let count = allocation_counter::measure(|| {
            for b in output.chunks_mut(block) {
                p.process(
                    &mut AudioBuffer::new(b, 1).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Default::default(),
                    },
                );
            }
        });
        assert_eq!(count.count_total, 0);
        (output, p.stats())
    }
    #[test]
    fn phase_locked_pitch_frequency_partition_unity_and_invalid() {
        let source = tone(24000);
        let (a, _) = pitch(&source, 12., 64, Default::default());
        let (b, _) = pitch(&source, 12., 257, Default::default());
        assert_eq!(a, b);
        assert!((hz(&a[4096..]) - 880.).abs() < 4.);
        let (unity, _) = pitch(&source, 0., 127, Default::default());
        for i in 2048..source.len() {
            assert!((unity[i] - source[i - 1024]).abs() < 2e-6);
        }
        let settings = PitchSettings {
            mix: 0.,
            ..Default::default()
        };
        let (dry, _) = pitch(&source, 7., 257, settings);
        assert_eq!(&dry[1024..], &source[..source.len() - 1024]);
        assert!(PitchShifter::new(24., Default::default()).is_err());
        let mut p = PitchShifter::new(7., Default::default()).unwrap();
        let mut samples = [1.; 16];
        p.process(
            &mut AudioBuffer::new(&mut samples, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        assert_eq!(samples, [0.; 16]);
        assert_eq!(p.stats().invalid_blocks, 1);
    }
    #[test]
    fn onsets_reset_phase_and_reset_clears_state() {
        let mut input = vec![0.; 24000];
        let source = tone(4000);
        input[8000..12000].copy_from_slice(&source);
        input[16000..20000].copy_from_slice(&source);
        let (output, stats) = pitch(&input, 7., 257, Default::default());
        assert!(stats.transient_channel_resets > 0);
        assert!(output.iter().all(|v| v.is_finite()));
        assert!(output.iter().map(|v| v.abs()).fold(0f32, f32::max) > 0.05);
        let mut p = PitchShifter::new(7., Default::default()).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 256,
        })
        .unwrap();
        let mut source = [0.2; 256];
        p.process(
            &mut AudioBuffer::new(&mut source, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        p.reset();
        let mut silence = [0.; 256];
        p.process(
            &mut AudioBuffer::new(&mut silence, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        assert!(silence.iter().all(|v| *v == 0.));
    }
    fn stretch(input: &[f32], ratio: f64, in_block: usize, out_block: usize) -> (Vec<f32>, u64) {
        let mut p = TimeStretcher::new(StretchSettings {
            channels: 2,
            ratio,
            window_frames: 512,
            search_frames: 64,
            max_block_frames: 1024,
            ..Default::default()
        })
        .unwrap();
        let mut output = Vec::with_capacity(((input.len() / 2) as f64 * ratio).ceil() as usize * 2);
        let mut tmp = vec![0.; out_block * 2];
        let mut offset = 0;
        let alloc = allocation_counter::measure(|| {
            while offset < input.len() / 2 {
                let end = (offset + in_block).min(input.len() / 2);
                let consumed = p.push(&input[offset * 2..end * 2]).unwrap();
                offset += consumed;
                let made = p.pull(&mut tmp, 8).unwrap();
                output.extend_from_slice(&tmp[..made * 2]);
                assert!(consumed > 0 || made > 0);
            }
            p.finish();
            while !p.is_drained() {
                let made = p.pull(&mut tmp, 8).unwrap();
                assert!(made > 0);
                output.extend_from_slice(&tmp[..made * 2]);
            }
        });
        assert_eq!(alloc.count_total, 0);
        assert_eq!(p.output_frames() as usize, output.len() / 2);
        (output, p.transient_locks())
    }
    #[test]
    fn wsola_duration_pitch_stereo_coherence_partition_and_no_alloc() {
        let source: Vec<_> = tone(12000)
            .into_iter()
            .flat_map(|s| [s, s * 0.25])
            .collect();
        for ratio in [0.5, 0.75, 1., 1.5, 2.] {
            let (a, _) = stretch(&source, ratio, 127, 64);
            let (b, _) = stretch(&source, ratio, 511, 257);
            assert_eq!(a, b, "ratio {ratio}");
            assert_eq!(a.len() / 2, (12000f64 * ratio).ceil() as usize);
            for frame in a.as_chunks::<2>().0 {
                assert_eq!(frame[1], frame[0] * 0.25);
            }
            let mono: Vec<_> = a.as_chunks::<2>().0.iter().map(|f| f[0]).collect();
            assert!(
                (hz(&mono[1024..mono.len() - 512]) - 440.).abs() < 5.,
                "ratio {ratio}"
            );
            if ratio == 1. {
                assert_eq!(a, source);
            }
        }
    }
    #[test]
    fn wsola_backpressure_finish_errors_and_reset() {
        let mut p = TimeStretcher::new(StretchSettings {
            channels: 1,
            ratio: 1.,
            window_frames: 256,
            search_frames: 32,
            max_block_frames: 128,
            ..Default::default()
        })
        .unwrap();
        let input = [0.1; 128];
        while p.input_space() >= 128 {
            assert_eq!(p.push(&input).unwrap(), 128);
        }
        assert_eq!(p.push(&input), Ok(0));
        let total = p.input_frames();
        assert!(p.push(&[f32::NAN]).is_err());
        assert_eq!(p.input_frames(), total);
        let mut output = [1.; 128];
        assert!(p.pull(&mut output, 65).is_err());
        assert_eq!(output, [0.; 128]);
        assert!(p.pull(&mut output, 1).unwrap() > 0);
        p.finish();
        assert!(p.push(&[]).is_err());
        while !p.is_drained() {
            assert!(p.pull(&mut output, 1).unwrap() > 0);
        }
        assert_eq!(p.output_frames(), total);
        p.reset();
        assert_eq!(p.input_frames(), 0);
        p.finish();
        assert!(p.is_drained());
        assert_eq!(p.pull(&mut output, 1), Ok(0));
        let burst: Vec<_> = (0..12000)
            .flat_map(|i| {
                let v = if (4000..4300).contains(&i) || (8000..8300).contains(&i) {
                    0.3
                } else {
                    0.
                };
                [v, v]
            })
            .collect();
        let (_, locks) = stretch(&burst, 1.5, 127, 64);
        assert!(locks > 0);
    }
}
#[cfg(test)]
mod linked_tests {
    use super::*;
    fn run(noisy: bool) -> Vec<f32> {
        let mut s = TimeStretcher::new(StretchSettings {
            channels: 4,
            ratio: 1.3,
            window_frames: 256,
            search_frames: 32,
            max_block_frames: 128,
            transient_ratio: Some(4.),
            ..Default::default()
        })
        .unwrap();
        s.set_reference_channel(Some(0)).unwrap();
        let mut input = vec![0.; 128 * 4];
        let mut output = input.clone();
        let mut result = vec![];
        for block in 0..32 {
            for (i, f) in input.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = ((block * 128 + i) as f32 * 0.071).sin() * 0.2;
                f.copy_from_slice(&[
                    x,
                    -x,
                    if noisy {
                        ((i * 13 + block * 7) as f32).sin() * 0.9
                    } else {
                        x * 0.5
                    },
                    x * 0.25,
                ]);
            }
            let mut at = 0;
            while at < 128 {
                let accepted = s.push(&input[at * 4..]).unwrap();
                at += accepted;
                let made = s.pull(&mut output, 8).unwrap();
                result.extend_from_slice(&output[..made * 4]);
                assert!(accepted > 0 || made > 0);
            }
            loop {
                let n = s.pull(&mut output, 8).unwrap();
                result.extend_from_slice(&output[..n * 4]);
                if n == 0 {
                    break;
                }
            }
        }
        assert!(s.set_reference_channel(Some(1)).is_err());
        s.finish();
        while !s.is_drained() {
            let n = s.pull(&mut output, 8).unwrap();
            assert!(n > 0);
            result.extend_from_slice(&output[..n * 4]);
        }
        result
    }
    #[test]
    fn selected_reference_owns_all_grain_decisions() {
        let a = run(false);
        let b = run(true);
        assert_eq!(a.len(), b.len());
        for (a, b) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
            assert_eq!(a[0], b[0]);
            assert_eq!(b[0], -b[1]);
            assert_eq!(b[0] * 0.25, b[3]);
        }
    }
}
