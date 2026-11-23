//! Frequency-guided partial tracking and complementary spectral masks.
use crate::advanced::spectral::Stft;
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub fundamental_hz: f32,
    pub harmonic_gain: f32,
    pub residual_gain: f32,
    pub width_bins: f32,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Partial {
    pub frequency_hz: f32,
    pub spectral_magnitude: f32,
}
struct Channel {
    stft: Stft,
    mask: Vec<f32>,
    tracks: Vec<Partial>,
}
pub struct Resynth {
    size: usize,
    count: usize,
    controls: Controls,
    spec: Option<ProcessSpec>,
    channels: Vec<Channel>,
}
impl Resynth {
    pub fn new(size: usize, count: usize, controls: Controls) -> Result<Self, Error> {
        if !size.is_power_of_two() || !(512..=8192).contains(&size) || !(1..=64).contains(&count) {
            return Err(Error::InvalidParameter);
        }
        let mut p = Self {
            size,
            count,
            controls,
            spec: None,
            channels: Vec::new(),
        };
        p.set_controls(controls)?;
        Ok(p)
    }
    pub fn set_controls(&mut self, c: Controls) -> Result<(), Error> {
        if !(25. ..=4000.).contains(&c.fundamental_hz)
            || !(0. ..=2.).contains(&c.harmonic_gain)
            || !(0. ..=2.).contains(&c.residual_gain)
            || !(1. ..=4.).contains(&c.width_bins)
        {
            return Err(Error::InvalidParameter);
        }
        self.controls = c;
        Ok(())
    }
    pub fn partials(&self, channel: usize) -> Option<&[Partial]> {
        self.channels.get(channel).map(|c| c.tracks.as_slice())
    }
}
impl AudioProcessor for Resynth {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels > 8 || !(8000. ..=192000.).contains(&s.sample_rate) {
            return Err(Error::InvalidSpec);
        }
        self.channels = (0..s.channels)
            .map(|_| {
                Ok(Channel {
                    stft: Stft::new(self.size, self.size / 4)?,
                    mask: vec![0.; self.size / 2 + 1],
                    tracks: vec![Partial::default(); self.count],
                })
            })
            .collect::<Result<_, Error>>()?;
        self.spec = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.clear();
            return;
        }
        let c = self.controls;
        let bin_hz = (s.sample_rate / self.size as f64) as f32;
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            for (x, state) in frame.iter_mut().zip(&mut self.channels) {
                let Channel { stft, mask, tracks } = state;
                *x = stft.tick(
                    if x.is_finite() {
                        x.clamp(-32., 32.)
                    } else {
                        0.
                    },
                    |bins| {
                        let nyquist = bins.len() / 2;
                        let spacing = c.fundamental_hz / bin_hz;
                        mask.fill(0.);
                        for (h, p) in tracks.iter_mut().enumerate() {
                            let expected = spacing * (h + 1) as f32;
                            if expected >= (nyquist - 1) as f32 || spacing < 2. {
                                *p = Partial::default();
                                continue;
                            }
                            let width = c.width_bins.min(spacing * 0.48);
                            let lo = ((expected - width).floor() as usize).max(1);
                            let hi = ((expected + width).ceil() as usize).min(nyquist - 1);
                            let k = (lo..=hi)
                                .max_by(|&i, &j| bins[i].norm_sqr().total_cmp(&bins[j].norm_sqr()))
                                .expect("nonempty local window");
                            let a = bins[k - 1].norm();
                            let m = bins[k].norm();
                            let z = bins[k + 1].norm();
                            let denom = a - 2. * m + z;
                            let shift = if denom.abs() > 1e-8 {
                                (0.5 * (a - z) / denom).clamp(-0.5, 0.5)
                            } else {
                                0.
                            };
                            let peak = k as f32 + shift;
                            let center = if p.spectral_magnitude > 1e-8 {
                                0.25 * p.frequency_hz / bin_hz + 0.75 * peak
                            } else {
                                peak
                            };
                            *p = Partial {
                                frequency_hz: center * bin_hz,
                                spectral_magnitude: m,
                            };
                            let first = ((center - width).floor() as usize).max(1);
                            let last = ((center + width).ceil() as usize).min(nyquist - 1);
                            for (i, v) in mask.iter_mut().enumerate().take(last + 1).skip(first) {
                                *v = v.max(
                                    (1. - ((i as f32 - center).abs() / width).powi(2)).max(0.),
                                );
                            }
                        }
                        let n = bins.len();
                        for (k, &harmonic) in mask.iter().enumerate() {
                            let gain =
                                c.residual_gain + (c.harmonic_gain - c.residual_gain) * harmonic;
                            bins[k] *= gain;
                            if k > 0 && k < nyquist {
                                bins[n - k] *= gain;
                            }
                        }
                    },
                );
            }
        }
    }
    fn reset(&mut self) {
        for c in &mut self.channels {
            c.stft.reset();
            c.tracks.fill(Partial::default());
            c.mask.fill(0.);
        }
    }
    fn latency_frames(&self) -> usize {
        self.size
    }
}
