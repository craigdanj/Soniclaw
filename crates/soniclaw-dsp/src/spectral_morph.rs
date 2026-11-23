//! Streaming cross-synthesis with the phase of one input and two magnitude controls.
use crate::advanced::spectral::{Complex32, Stft};
use soniclaw_core::{AudioBuffer, Error, ProcessSpec};

#[derive(Clone, Copy, Debug)]
pub struct Morph {
    /// Blend between broad amplitude envelopes.
    pub envelope: f32,
    /// Blend between magnitudes divided by their own envelopes.
    pub fine_structure: f32,
}
impl Morph {
    fn validated(self) -> Result<Self, Error> {
        if !(0. ..=1.).contains(&self.envelope) || !(0. ..=1.).contains(&self.fine_structure) {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }
}

struct Spectrum {
    magnitude: Vec<f32>,
    sums: Vec<f64>,
}
impl Spectrum {
    fn new(bins: usize) -> Self {
        Self {
            magnitude: vec![0.; bins],
            sums: vec![0.; bins + 1],
        }
    }

    fn update(&mut self, bins: &[Complex32]) {
        self.sums[0] = 0.;
        for (i, (magnitude, value)) in self.magnitude.iter_mut().zip(bins).enumerate() {
            *magnitude = value.norm();
            self.sums[i + 1] = self.sums[i] + f64::from(*magnitude);
        }
    }

    fn envelope(&self, bin: usize, width: usize, floor: f32) -> f32 {
        let low = bin.saturating_sub(width);
        let high = self.magnitude.len().min(bin + width + 1);
        (((self.sums[high] - self.sums[low]) / (high - low) as f64) as f32).max(floor)
    }
}

struct Channel {
    carrier: Stft,
    modulator: Stft,
    a: Spectrum,
    b: Spectrum,
}

pub struct SpectralMorph {
    spec: ProcessSpec,
    size: usize,
    envelope_width: usize,
    controls: Morph,
    channels: Vec<Channel>,
}

impl SpectralMorph {
    pub fn new(
        spec: ProcessSpec,
        size: usize,
        envelope_bins: usize,
        controls: Morph,
    ) -> Result<Self, Error> {
        let spec = spec.validate()?;
        let controls = controls.validated()?;
        if !(8_000. ..=192_000.).contains(&spec.sample_rate)
            || spec.channels > 8
            || !(256..=8192).contains(&size)
            || !size.is_power_of_two()
            || !(2..=(size / 4).min(128)).contains(&envelope_bins)
        {
            return Err(Error::InvalidSpec);
        }
        let channels = (0..spec.channels)
            .map(|_| {
                Ok(Channel {
                    carrier: Stft::new(size, size / 4)?,
                    modulator: Stft::new(size, size / 4)?,
                    a: Spectrum::new(size / 2 + 1),
                    b: Spectrum::new(size / 2 + 1),
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            spec,
            size,
            envelope_width: envelope_bins,
            controls,
            channels,
        })
    }

    pub fn set_morph(&mut self, controls: Morph) -> Result<(), Error> {
        self.controls = controls.validated()?;
        Ok(())
    }

    pub fn latency_frames(&self) -> usize {
        self.size
    }

    /// Replace carrier with the morph result. Inputs must be sample aligned.
    pub fn process(
        &mut self,
        carrier: &mut AudioBuffer<'_>,
        modulator: &[f32],
    ) -> Result<(), Error> {
        let channels = self.spec.channels;
        if carrier.channels() != channels
            || carrier.frames() > self.spec.max_block_frames
            || modulator.len() != carrier.samples().len()
        {
            return Err(Error::InvalidBuffer);
        }
        let controls = self.controls;
        let width = self.envelope_width;
        let half = self.size / 2;
        for (carrier, modulator) in carrier
            .samples_mut()
            .chunks_exact_mut(channels)
            .zip(modulator.chunks_exact(channels))
        {
            for ((x, &side), state) in carrier.iter_mut().zip(modulator).zip(&mut self.channels) {
                let Channel {
                    carrier,
                    modulator,
                    a,
                    b,
                } = state;
                modulator.tick_analysis(finite_sample(side), |bins| b.update(bins));
                *x = carrier.tick(finite_sample(*x), |bins| {
                    if controls.envelope == 0. && controls.fine_structure == 0. {
                        return;
                    }
                    a.update(bins);
                    let floor = a
                        .magnitude
                        .iter()
                        .chain(&b.magnitude)
                        .copied()
                        .fold(1e-4, f32::max)
                        * 1e-6;
                    for k in 0..=half {
                        let amplitude = a.magnitude[k];
                        let ea = a.envelope(k, width, floor);
                        let eb = b.envelope(k, width, floor);
                        let envelope = (ea.ln() + (eb.ln() - ea.ln()) * controls.envelope).exp();
                        let fine_a = amplitude / ea;
                        let fine_b = b.magnitude[k] / eb;
                        let fine = fine_a + (fine_b - fine_a) * controls.fine_structure;
                        let target = (envelope * fine).min(amplitude.max(b.magnitude[k]) * 8.);
                        let value = if amplitude <= floor {
                            Complex32::new(target, 0.)
                        } else {
                            bins[k] * (target / amplitude)
                        };
                        if k == 0 || k == half {
                            bins[k] = Complex32::new(value.re, 0.);
                        } else {
                            bins[k] = value;
                            bins[2 * half - k] = value.conj();
                        }
                    }
                });
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.carrier.reset();
            ch.modulator.reset();
        }
    }
}

fn finite_sample(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-16., 16.)
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_aligns_and_envelope_changes_while_remaining_finite() {
        let spec = ProcessSpec {
            sample_rate: 48_000.,
            max_block_frames: 512,
            channels: 1,
        };
        let signal: Vec<_> = (0..4096)
            .map(|i| ((i as f32) * 0.137).sin() * 0.25)
            .collect();
        let modulator: Vec<_> = signal.iter().map(|x| x * 0.2).collect();
        for (morph, gain) in [(0., 1.), (1., 0.2)] {
            let mut dsp = SpectralMorph::new(
                spec,
                512,
                8,
                Morph {
                    envelope: morph,
                    fine_structure: 0.,
                },
            )
            .unwrap();
            let mut pcm = signal.clone();
            let a = allocation_counter::measure(|| {
                for (x, s) in pcm.chunks_mut(487).zip(modulator.chunks(487)) {
                    dsp.process(&mut AudioBuffer::new(x, 1).unwrap(), s)
                        .unwrap();
                }
            });
            assert_eq!(a.count_total, 0);
            assert!(pcm[..512].iter().all(|&x| x == 0.));
            for i in 512..pcm.len() {
                assert!((pcm[i] - signal[i - 512] * gain).abs() < 0.0002);
            }
        }
    }
}
