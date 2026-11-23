//! Phase-aware freeze, robotisation, Hilbert frequency shifting and phase-vocoder
//! pitch/time tools. Fixed-I/O pitch shifting is real-time; time stretch is offline.
use crate::advanced::{
    clean, spec,
    spectral::{window, Complex32, FftProcessor, Stft, WindowKind},
    LiveDsp,
};
use soniclaw_core::*;
use std::f32::consts::{PI, TAU};
fn wrap(x: f32) -> f32 {
    (x + PI).rem_euclid(TAU) - PI
}
struct SpectralChannel {
    stft: Stft,
    previous: Vec<f32>,
    phase: Vec<f32>,
    magnitude: Vec<f32>,
    frequency: Vec<f32>,
    sum: Vec<f32>,
    weighted: Vec<f32>,
    frames: usize,
    held: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectralMode {
    Freeze,
    Robot,
    PitchShift,
}
pub struct SpectralEffect {
    mode: SpectralMode,
    channels: Vec<SpectralChannel>,
    dry: Vec<f32>,
    cursor: usize,
    size: usize,
    hop: usize,
    amount: f32,
    mix: f32,
}
impl SpectralEffect {
    pub fn new(mode: SpectralMode) -> Self {
        Self {
            mode,
            channels: vec![],
            dry: vec![],
            cursor: 0,
            size: 1024,
            hop: 256,
            amount: 0.0,
            mix: 1.0,
        }
    }
    pub fn set(&mut self, amount: f32, mix: f32) -> Result<(), Error> {
        let range = if self.mode == SpectralMode::PitchShift {
            -24.0..=24.0
        } else {
            0.0..=1.0
        };
        if !amount.is_finite()
            || !range.contains(&amount)
            || !mix.is_finite()
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        self.amount = amount;
        self.mix = mix;
        Ok(())
    }
}
impl LiveDsp for SpectralEffect {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for SpectralEffect {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        let n = self.size / 2 + 1;
        self.channels = (0..s.channels)
            .map(|_| {
                Ok(SpectralChannel {
                    stft: Stft::new(self.size, self.hop)?,
                    previous: vec![0.0; n],
                    phase: vec![0.0; n],
                    magnitude: vec![0.0; n],
                    frequency: vec![0.0; n],
                    sum: vec![0.0; n],
                    weighted: vec![0.0; n],
                    frames: 0,
                    held: false,
                })
            })
            .collect::<Result<_, Error>>()?;
        self.dry = vec![0.0; self.size * s.channels];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let channels = b.channels();
        if self.channels.len() != channels {
            b.clear();
            return;
        }
        let mode = self.mode;
        let amount = self.amount;
        let size = self.size;
        let hop = self.hop;
        for frame in b.samples_mut().chunks_exact_mut(channels) {
            for (c, x) in frame.iter_mut().enumerate() {
                let ch = &mut self.channels[c];
                ch.frames = ch.frames.saturating_add(1);
                let wet = ch.stft.tick(*x, |bins| {
                    let half = size / 2;
                    let eligible = ch.frames >= size;
                    match mode {
                        SpectralMode::Robot => {
                            if amount > 0.5 {
                                for bin in bins.iter_mut() {
                                    *bin = Complex32::new(bin.norm(), 0.0);
                                }
                            }
                        }
                        SpectralMode::Freeze => {
                            let hold = amount > 0.5 && eligible;
                            for k in 0..=half {
                                let phase = bins[k].arg();
                                let omega = TAU * k as f32 / size as f32;
                                let f = omega
                                    + wrap(phase - ch.previous[k] - omega * hop as f32)
                                        / hop as f32;
                                ch.previous[k] = phase;
                                if hold {
                                    if !ch.held {
                                        ch.magnitude[k] = bins[k].norm();
                                        ch.frequency[k] = f;
                                        ch.phase[k] = phase;
                                    } else {
                                        ch.phase[k] =
                                            wrap(ch.phase[k] + ch.frequency[k] * hop as f32);
                                    }
                                    bins[k] = Complex32::from_polar(ch.magnitude[k], ch.phase[k]);
                                }
                            }
                            ch.held = hold;
                            if hold {
                                bins[0].im = 0.0;
                                bins[half].im = 0.0;
                                for k in 1..half {
                                    bins[size - k] = bins[k].conj();
                                }
                            }
                        }
                        SpectralMode::PitchShift => {
                            let ratio = 2.0_f32.powf(amount / 12.0);
                            ch.sum.fill(0.0);
                            ch.weighted.fill(0.0);
                            for k in 0..=half {
                                let phase = bins[k].arg();
                                let omega = TAU * k as f32 / size as f32;
                                let frequency = omega
                                    + wrap(phase - ch.previous[k] - omega * hop as f32)
                                        / hop as f32;
                                ch.previous[k] = phase;
                                let destination = k as f32 * ratio;
                                let a = destination.floor() as usize;
                                let frac = destination.fract();
                                let mag = bins[k].norm();
                                for (j, w) in [(a, 1.0 - frac), (a + 1, frac)] {
                                    if j <= half {
                                        ch.sum[j] += mag * w;
                                        ch.weighted[j] += mag * w * frequency * ratio;
                                    }
                                }
                            }
                            if amount.abs() > 1e-6 {
                                for k in 0..=half {
                                    let frequency = if ch.sum[k] > 1e-12 {
                                        ch.weighted[k] / ch.sum[k]
                                    } else {
                                        TAU * k as f32 / size as f32
                                    };
                                    ch.phase[k] = wrap(ch.phase[k] + frequency * hop as f32);
                                    bins[k] = Complex32::from_polar(ch.sum[k], ch.phase[k]);
                                }
                                bins[0].im = 0.0;
                                bins[half].im = 0.0;
                                for k in 1..half {
                                    bins[size - k] = bins[k].conj();
                                }
                            } else {
                                for k in 0..=half {
                                    ch.phase[k] = bins[k].arg();
                                }
                            }
                        }
                    }
                });
                let idx = self.cursor * channels + c;
                let dry = self.dry[idx];
                self.dry[idx] = *x;
                *x = dry * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % size;
        }
    }
    fn reset(&mut self) {
        self.dry.fill(0.0);
        self.cursor = 0;
        for c in &mut self.channels {
            c.stft.reset();
            c.previous.fill(0.0);
            c.phase.fill(0.0);
            c.magnitude.fill(0.0);
            c.frequency.fill(0.0);
            c.sum.fill(0.0);
            c.weighted.fill(0.0);
            c.held = false;
            c.frames = 0;
        }
    }
    fn latency_frames(&self) -> usize {
        self.size
    }
}
pub struct FrequencyShifter {
    history: Vec<[f32; 63]>,
    taps: [f32; 63],
    cursor: usize,
    sr: f32,
    phase: f32,
    shift: f32,
    mix: f32,
}
impl Default for FrequencyShifter {
    fn default() -> Self {
        let taps = std::array::from_fn(|i| {
            let n = i as isize - 31;
            if n == 0 || n % 2 == 0 {
                0.0
            } else {
                2.0 / (PI * n as f32)
                    * (0.42 - 0.5 * (TAU * i as f32 / 62.0).cos()
                        + 0.08 * (2.0 * TAU * i as f32 / 62.0).cos())
            }
        });
        Self {
            history: vec![],
            taps,
            cursor: 0,
            sr: 48000.0,
            phase: 0.0,
            shift: 0.0,
            mix: 1.0,
        }
    }
}
impl FrequencyShifter {
    pub fn set(&mut self, shift_hz: f32, mix: f32) -> Result<(), Error> {
        if !shift_hz.is_finite()
            || !(-2000.0..=2000.0).contains(&shift_hz)
            || !mix.is_finite()
            || !(0.0..=1.0).contains(&mix)
        {
            return Err(Error::InvalidParameter);
        }
        self.shift = shift_hz;
        self.mix = mix;
        Ok(())
    }
}
impl LiveDsp for FrequencyShifter {
    fn update(&mut self, v: &[f32]) {
        let _ = self.set(v[0], v[1]);
    }
}
impl AudioProcessor for FrequencyShifter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate as f32;
        self.history = vec![[0.0; 63]; s.channels];
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        if self.history.len() != ch {
            b.clear();
            return;
        }
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            let (cs, sn) = (self.phase.cos(), self.phase.sin());
            for (x, h) in frame.iter_mut().zip(&mut self.history) {
                h[self.cursor] = *x;
                let direct = h[(self.cursor + 63 - 31) % 63];
                let mut q = 0.0;
                for (i, t) in self.taps.iter().enumerate() {
                    q += t * h[(self.cursor + 63 - i) % 63];
                }
                let wet = direct * cs - q * sn;
                *x = direct * (1.0 - self.mix) + wet * self.mix;
            }
            self.cursor = (self.cursor + 1) % 63;
            self.phase = (self.phase + TAU * self.shift / self.sr).rem_euclid(TAU);
        }
    }
    fn reset(&mut self) {
        self.history.fill([0.0; 63]);
        self.cursor = 0;
        self.phase = 0.0;
    }
    fn latency_frames(&self) -> usize {
        31
    }
}
/// Allocating offline phase vocoder. Output length is ceil(input_frames*ratio).
/// Basic phase propagation; no formant preservation or transient locking.
pub fn time_stretch_interleaved(
    input: &[f32],
    channels: usize,
    ratio: f64,
    size: usize,
) -> Result<Vec<f32>, Error> {
    if !(1..=2).contains(&channels)
        || !input.len().is_multiple_of(channels)
        || input.len() > 8 * 1024 * 1024
        || input.iter().any(|x| !x.is_finite())
        || !ratio.is_finite()
        || !(0.25..=4.0).contains(&ratio)
        || !size.is_power_of_two()
        || !(256..=4096).contains(&size)
    {
        return Err(Error::InvalidSpec);
    }
    if ratio == 1.0 {
        return Ok(input.to_vec());
    }
    let frames = input.len() / channels;
    let length = (frames as f64 * ratio).ceil() as usize;
    let count = length.checked_mul(channels).ok_or(Error::InvalidBuffer)?;
    if count > 32 * 1024 * 1024 {
        return Err(Error::InvalidBuffer);
    }
    let mut output = vec![0.0; count];
    if frames == 0 {
        return Ok(output);
    }
    let hop = size / 4;
    let w: Vec<_> = window(WindowKind::Hann, size)?
        .into_iter()
        .map(|x| x.max(0.0).sqrt())
        .collect();
    let mut fft = FftProcessor::new(size)?;
    let mut bins = vec![Complex32::default(); size];
    let mut previous = vec![0.0; size / 2 + 1];
    let mut phase = vec![0.0; size / 2 + 1];
    let mut weight = vec![0.0; length];
    for channel in 0..channels {
        previous.fill(0.0);
        phase.fill(0.0);
        weight.fill(0.0);
        let mut source = -(size as isize) + hop as isize;
        let mut last_destination = (source as f64 * ratio).round() as isize;
        let mut first = true;
        while source < frames as isize {
            for i in 0..size {
                let index = source + i as isize;
                bins[i] = Complex32::new(
                    if index >= 0 && (index as usize) < frames {
                        input[index as usize * channels + channel] * w[i]
                    } else {
                        0.0
                    },
                    0.0,
                );
            }
            fft.forward(&mut bins)?;
            let destination = (source as f64 * ratio).round() as isize;
            let synthesis_hop = (destination - last_destination) as f32;
            for k in 0..=size / 2 {
                let p = bins[k].arg();
                let omega = TAU * k as f32 / size as f32;
                let frequency = omega + wrap(p - previous[k] - omega * hop as f32) / hop as f32;
                previous[k] = p;
                if first {
                    phase[k] = p;
                } else {
                    phase[k] = wrap(phase[k] + frequency * synthesis_hop);
                }
                bins[k] = Complex32::from_polar(bins[k].norm(), phase[k]);
            }
            bins[0].im = 0.0;
            bins[size / 2].im = 0.0;
            for k in 1..size / 2 {
                bins[size - k] = bins[k].conj();
            }
            fft.inverse(&mut bins)?;
            for i in 0..size {
                let index = destination + i as isize;
                if index >= 0 && (index as usize) < length {
                    let j = index as usize;
                    output[j * channels + channel] += bins[i].re * w[i];
                    weight[j] += w[i] * w[i];
                }
            }
            first = false;
            last_destination = destination;
            source += hop as isize;
        }
        for (i, weight) in weight.iter().enumerate() {
            output[i * channels + channel] = if *weight > 1e-8 {
                clean(output[i * channels + channel] / weight)
            } else {
                0.0
            };
        }
    }
    Ok(output)
}
pub fn pitch_shift_interleaved(
    input: &[f32],
    channels: usize,
    rate: f64,
    semitones: f32,
) -> Result<Vec<f32>, Error> {
    if !semitones.is_finite() || !(-24.0..=24.0).contains(&semitones) {
        return Err(Error::InvalidParameter);
    }
    let ratio = 2.0_f64.powf(semitones as f64 / 12.0);
    let stretched = time_stretch_interleaved(input, channels, ratio, 1024)?;
    let mut output = crate::resample::resample_interleaved(
        &stretched,
        channels,
        rate * ratio,
        rate,
        crate::resample::ResampleQuality::SincBest,
    )?;
    output.resize(input.len(), 0.0);
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn crossings(d: &[f32]) -> usize {
        d.windows(2).filter(|x| x[0] <= 0.0 && x[1] > 0.0).count()
    }
    #[test]
    fn offline_stretch_duration_and_pitch() {
        let input: Vec<_> = (0..24000)
            .map(|i| 0.2 * (TAU * 440.0 * i as f32 / 48000.0).sin())
            .collect();
        let stretched = time_stretch_interleaved(&input, 1, 2.0, 1024).unwrap();
        assert_eq!(stretched.len(), 48000);
        assert!((crossings(&stretched[4800..43200]) as f32 - 352.0).abs() < 5.0);
        let shifted = pitch_shift_interleaved(&input, 1, 48000.0, 12.0).unwrap();
        assert_eq!(shifted.len(), 24000);
        assert!((crossings(&shifted[4800..19200]) as f32 - 264.0).abs() < 8.0);
    }
    #[test]
    fn hilbert_frequency_shift_translates_tone() {
        let mut p = FrequencyShifter::default();
        p.set(200.0, 1.0).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 48000,
            channels: 1,
        })
        .unwrap();
        let mut d: Vec<_> = (0..48000)
            .map(|i| (TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.2)
            .collect();
        p.process(
            &mut AudioBuffer::new(&mut d, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert!((crossings(&d[4800..43200]) as f32 - 960.0).abs() < 2.0);
    }
    #[test]
    fn freeze_continues_after_input_silence() {
        let mut p = SpectralEffect::new(SpectralMode::Freeze);
        p.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 4800,
            channels: 1,
        })
        .unwrap();
        let mut tone: Vec<_> = (0..4800)
            .map(|i| (TAU * 750.0 * i as f32 / 48000.0).sin() * 0.1)
            .collect();
        let ctx = ProcessContext {
            events: &[],
            transport: Transport::default(),
        };
        p.process(&mut AudioBuffer::new(&mut tone, 1).unwrap(), &ctx);
        p.set(1.0, 1.0).unwrap();
        p.process(&mut AudioBuffer::new(&mut tone, 1).unwrap(), &ctx);
        let mut silent = vec![0.0; 4800];
        p.process(&mut AudioBuffer::new(&mut silent, 1).unwrap(), &ctx);
        assert!(silent[2048..].iter().any(|x| x.abs() > 0.01));
        p.reset();
        silent.fill(0.0);
        p.process(&mut AudioBuffer::new(&mut silent, 1).unwrap(), &ctx);
        assert!(silent.iter().all(|x| *x == 0.0));
    }
}
