//! Offline, allocating measurements. Coherent spectra assume bin-centred tones and no window.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use soniclaw_dsp::advanced::spectral::{Complex32, FftProcessor};
use std::collections::{BTreeMap, BTreeSet};
fn db_power(v: f64) -> f64 {
    10. * v.max(1e-30).log10()
}
#[derive(Clone, Debug, Serialize)]
pub struct Statistics {
    pub dc: f64,
    pub rms: f64,
    pub ac_rms: f64,
    pub peak: f64,
    pub samples_at_or_above_unity: usize,
}
pub fn statistics(pcm: &[f32]) -> Result<Statistics, Error> {
    if pcm.is_empty()
        || pcm.len() > 4 * 1024 * 1024
        || pcm.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
    {
        return Err(Error::InvalidBuffer);
    }
    let dc = pcm.iter().map(|v| *v as f64).sum::<f64>() / pcm.len() as f64;
    let power = pcm.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / pcm.len() as f64;
    let ac = pcm.iter().map(|v| (*v as f64 - dc).powi(2)).sum::<f64>() / pcm.len() as f64;
    Ok(Statistics {
        dc,
        rms: power.sqrt(),
        ac_rms: ac.sqrt(),
        peak: pcm.iter().map(|v| (*v as f64).abs()).fold(0., f64::max),
        samples_at_or_above_unity: pcm.iter().filter(|v| v.abs() >= 1.).count(),
    })
}
pub fn coherent_sine(
    size: usize,
    bin: usize,
    amplitude: f32,
    phase: f64,
) -> Result<Vec<f32>, Error> {
    if !size.is_power_of_two()
        || !(16..=32768).contains(&size)
        || bin == 0
        || bin >= size / 2
        || !amplitude.is_finite()
        || amplitude.abs() > 1.
        || !phase.is_finite()
    {
        return Err(Error::InvalidSpec);
    }
    Ok((0..size)
        .map(|i| {
            (amplitude as f64
                * (std::f64::consts::TAU * bin as f64 * i as f64 / size as f64 + phase).sin())
                as f32
        })
        .collect())
}
pub fn log_sweep(
    rate: u32,
    frames: usize,
    start: f64,
    end: f64,
    amplitude: f32,
) -> Result<Vec<f32>, Error> {
    if !(8000..=192000).contains(&rate)
        || !(16..=1024 * 1024).contains(&frames)
        || !start.is_finite()
        || !end.is_finite()
        || start <= 0.
        || start >= end
        || end >= rate as f64 * 0.5
        || !amplitude.is_finite()
        || amplitude.abs() > 1.
    {
        return Err(Error::InvalidSpec);
    }
    let duration = frames as f64 / rate as f64;
    let k = (end / start).ln();
    Ok((0..frames)
        .map(|i| {
            (amplitude as f64
                * (std::f64::consts::TAU * start * duration / k
                    * ((i as f64 / frames as f64 * k).exp() - 1.))
                    .sin()) as f32
        })
        .collect())
}
pub fn deterministic_noise(frames: usize, amplitude: f32, seed: u32) -> Result<Vec<f32>, Error> {
    if frames == 0
        || frames > 1024 * 1024
        || !amplitude.is_finite()
        || !(0.0..=1.).contains(&amplitude)
    {
        return Err(Error::InvalidSpec);
    }
    let mut state = seed;
    Ok((0..frames)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            ((state as f64 / u32::MAX as f64 * 2. - 1.) * amplitude as f64) as f32
        })
        .collect())
}
pub struct Spectrum {
    size: usize,
    power: Vec<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Distortion {
    pub fundamental_rms: f64,
    pub thd_db: f64,
    pub thdn_db: f64,
    pub snr_db: f64,
    pub noise_dbfs: f64,
}
impl Spectrum {
    pub fn coherent(pcm: &[f32]) -> Result<Self, Error> {
        statistics(pcm)?;
        let n = pcm.len();
        let mut fft = FftProcessor::new(n)?;
        let mut bins: Vec<_> = pcm.iter().map(|v| Complex32::new(*v, 0.)).collect();
        fft.forward(&mut bins)?;
        let power = bins[..=n / 2]
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let k = if i == 0 || i == n / 2 { 1. } else { 2. };
                (b.re as f64 * b.re as f64 + b.im as f64 * b.im as f64) * k / (n as f64 * n as f64)
            })
            .collect();
        Ok(Self { size: n, power })
    }
    pub fn size(&self) -> usize {
        self.size
    }
    fn indices(&self, bins: &[usize]) -> Result<BTreeSet<usize>, Error> {
        if bins.is_empty()
            || bins.len() > 1024
            || bins.iter().any(|b| *b == 0 || *b > self.size / 2)
        {
            return Err(Error::InvalidParameter);
        }
        let set: BTreeSet<_> = bins.iter().copied().collect();
        if set.len() != bins.len() {
            return Err(Error::InvalidParameter);
        }
        Ok(set)
    }
    pub fn ratio_db(&self, reference: &[usize], measured: &[usize]) -> Result<f64, Error> {
        let r = self.indices(reference)?;
        let m = self.indices(measured)?;
        if !r.is_disjoint(&m) {
            return Err(Error::InvalidParameter);
        }
        let p = r.iter().map(|i| self.power[*i]).sum::<f64>();
        if p < 1e-30 {
            return Err(Error::InvalidBuffer);
        }
        Ok(db_power(m.iter().map(|i| self.power[*i]).sum::<f64>() / p))
    }
    pub fn distortion(
        &self,
        fundamentals: &[usize],
        max_harmonic: usize,
    ) -> Result<Distortion, Error> {
        if !(2..=64).contains(&max_harmonic) {
            return Err(Error::InvalidParameter);
        }
        let fundamental = self.indices(fundamentals)?;
        let mut harmonics = BTreeSet::new();
        for f in &fundamental {
            for h in 2..=max_harmonic {
                let bin = f * h;
                if bin <= self.size / 2 && !fundamental.contains(&bin) {
                    harmonics.insert(bin);
                }
            }
        }
        let signal = fundamental.iter().map(|i| self.power[*i]).sum::<f64>();
        if signal < 1e-30 {
            return Err(Error::InvalidBuffer);
        }
        let harmonic = harmonics.iter().map(|i| self.power[*i]).sum::<f64>();
        let residual = self
            .power
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, _)| !fundamental.contains(i))
            .map(|(_, v)| *v)
            .sum::<f64>();
        let noise = self
            .power
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(i, _)| !fundamental.contains(i) && !harmonics.contains(i))
            .map(|(_, v)| *v)
            .sum::<f64>();
        Ok(Distortion {
            fundamental_rms: signal.sqrt(),
            thd_db: db_power(harmonic / signal),
            thdn_db: db_power(residual / signal),
            snr_db: -db_power(noise / signal),
            noise_dbfs: db_power(noise),
        })
    }
}
pub fn alias_bin(harmonic_bin: usize, fft_size: usize) -> Result<usize, Error> {
    if !fft_size.is_power_of_two() || !(16..=32768).contains(&fft_size) {
        return Err(Error::InvalidSpec);
    }
    let b = harmonic_bin % fft_size;
    Ok(b.min(fft_size - b))
}
/// Prepare/render a mono processor from a known signal. Processing errors represented as silence need product-specific thresholds.
pub fn render(
    processor: &mut dyn AudioProcessor,
    input: &[f32],
    rate: f64,
    block: usize,
) -> Result<Vec<f32>, Error> {
    statistics(input)?;
    if !rate.is_finite() || !(8000.0..=192000.).contains(&rate) || !(1..=8192).contains(&block) {
        return Err(Error::InvalidSpec);
    }
    processor.prepare(ProcessSpec {
        sample_rate: rate,
        channels: 1,
        max_block_frames: block,
    })?;
    let mut out = input.to_vec();
    for (i, pcm) in out.chunks_mut(block).enumerate() {
        processor.process(
            &mut AudioBuffer::new(pcm, 1)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: (i * block) as u64,
                    ..Default::default()
                },
            },
        );
    }
    statistics(&out)?;
    Ok(out)
}
#[derive(Clone, Debug, Serialize)]
pub struct Response {
    pub frequency_hz: f64,
    pub gain_db: f64,
}
pub fn response(
    factory: impl Fn() -> Box<dyn AudioProcessor>,
    rate: u32,
    frequencies: &[f64],
    warmup: usize,
    measure: usize,
) -> Result<Vec<Response>, Error> {
    if !(8000..=192000).contains(&rate)
        || frequencies.is_empty()
        || frequencies.len() > 64
        || measure == 0
        || measure > rate as usize * 4
        || warmup > rate as usize * 4
        || frequencies
            .iter()
            .any(|f| !f.is_finite() || *f <= 0. || *f >= rate as f64 * 0.5)
    {
        return Err(Error::InvalidSpec);
    }
    let mut result = Vec::new();
    for f in frequencies {
        let input: Vec<_> = (0..warmup + measure)
            .map(|i| (0.25 * (std::f64::consts::TAU * f * i as f64 / rate as f64).sin()) as f32)
            .collect();
        let out = render(factory().as_mut(), &input, rate as f64, 256)?;
        let a = statistics(&input[warmup..])?.ac_rms;
        let b = statistics(&out[warmup..])?.ac_rms;
        if a < 1e-15 {
            return Err(Error::InvalidBuffer);
        }
        result.push(Response {
            frequency_hz: *f,
            gain_db: 20. * (b / a).max(1e-15).log10(),
        });
    }
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    pub name: String,
    pub minimum: f64,
    pub maximum: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub schema: u32,
    pub limits: Vec<Limit>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub value: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub passed: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub checks: Vec<Check>,
}
impl Report {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.passed)
    }
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
}
impl Suite {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1 || self.limits.is_empty() || self.limits.len() > 256 {
            return Err(Error::InvalidParameter);
        }
        let mut names = BTreeSet::new();
        for l in &self.limits {
            if l.name.is_empty()
                || l.name.len() > 128
                || !names.insert(&l.name)
                || !l.minimum.is_finite()
                || !l.maximum.is_finite()
                || l.minimum > l.maximum
            {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
    pub fn from_json(s: &str) -> Result<Self, Error> {
        if s.len() > 65536 {
            return Err(Error::InvalidParameter);
        }
        let suite: Self = serde_json::from_str(s).map_err(|_| Error::InvalidParameter)?;
        suite.validate()?;
        Ok(suite)
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn evaluate(&self, values: &BTreeMap<String, f64>) -> Result<Report, Error> {
        self.validate()?;
        if values.len() != self.limits.len() {
            return Err(Error::InvalidParameter);
        }
        let mut checks = Vec::new();
        for l in &self.limits {
            let value = *values.get(&l.name).ok_or(Error::InvalidParameter)?;
            if !value.is_finite() {
                return Err(Error::InvalidBuffer);
            }
            checks.push(Check {
                name: l.name.clone(),
                value,
                minimum: l.minimum,
                maximum: l.maximum,
                passed: value >= l.minimum && value <= l.maximum,
            });
        }
        Ok(Report { schema: 1, checks })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibrated_harmonic_noise_parseval_and_bins() {
        let mut x = coherent_sine(8192, 113, 0.5, 0.).unwrap();
        let h = coherent_sine(8192, 226, 0.005, 0.3).unwrap();
        for (a, b) in x.iter_mut().zip(h) {
            *a += b;
        }
        let spectrum = Spectrum::coherent(&x).unwrap();
        let d = spectrum.distortion(&[113], 12).unwrap();
        assert!((d.thd_db + 40.).abs() < 0.001);
        let measured = spectrum.power.iter().sum::<f64>();
        assert!((measured - statistics(&x).unwrap().rms.powi(2)).abs() < 1e-8);
        assert_eq!(alias_bin(5700, 8192), Ok(2492));
        assert!(spectrum.ratio_db(&[113], &[113]).is_err());
        let noise = deterministic_noise(8192, 0.001, 42).unwrap();
        for (a, b) in x.iter_mut().zip(noise) {
            *a += b;
        }
        let d = Spectrum::coherent(&x)
            .unwrap()
            .distortion(&[113], 12)
            .unwrap();
        assert!((54.0..58.).contains(&d.snr_db));
    }
    #[test]
    fn sweep_statistics_threshold_failure_and_invalid_signals() {
        let sweep = log_sweep(48000, 48000, 20., 20000., 0.5).unwrap();
        assert!(statistics(&sweep).unwrap().rms > 0.3);
        assert!(statistics(&[f32::INFINITY]).is_err());
        let suite = Suite {
            schema: 1,
            limits: vec![Limit {
                name: "gain_db".into(),
                minimum: -0.1,
                maximum: 0.1,
            }],
        };
        let suite = Suite::from_json(&suite.to_json().unwrap()).unwrap();
        let report = suite
            .evaluate(&BTreeMap::from([("gain_db".into(), -1.)]))
            .unwrap();
        assert!(!report.passed());
        assert!(report.to_json().unwrap().contains("false"));
        assert!(suite.evaluate(&BTreeMap::new()).is_err());
        assert!(Spectrum::coherent(&[0.; 17]).is_err());
    }
}
