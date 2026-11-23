//! Offline two-pass loudness-targeted mastering for mono and stereo PCM.
use crate::{
    advanced::{
        analysis::{LoudnessMeter, TruePeakDetector},
        dynamics::LookaheadLimiter,
    },
    resample::{resample_interleaved, ResampleQuality},
};
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

pub const MAX_FRAMES: usize = 16_777_216;
const BLOCK: usize = 2048;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub target_lufs: f64,
    pub peak_ceiling_dbtp: f32,
    pub max_boost_db: f64,
    pub lookahead_ms: f32,
    pub release_ms: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            target_lufs: -14.,
            peak_ceiling_dbtp: -1.,
            max_boost_db: 24.,
            lookahead_ms: 5.,
            release_ms: 90.,
        }
    }
}
impl Options {
    fn validate(self) -> Result<(), Error> {
        if !(-45. ..=-6.).contains(&self.target_lufs)
            || !(-20. ..=-0.1).contains(&self.peak_ceiling_dbtp)
            || !(0. ..=32.).contains(&self.max_boost_db)
            || !(1. ..=20.).contains(&self.lookahead_ms)
            || !(1. ..=2000.).contains(&self.release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Measurements {
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f32>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Report {
    pub input: Measurements,
    pub output: Measurements,
    pub requested_lufs: f64,
    pub requested_ceiling_dbtp: f32,
    pub level_gain_db: f64,
    pub final_peak_trim_db: f32,
    /// Within 0.25 LU and at or below the configured measured peak ceiling.
    pub met_target: bool,
    pub limiter_latency_frames: usize,
}

#[derive(Debug)]
pub struct Master {
    pub samples: Vec<f32>,
    pub report: Report,
}

#[derive(Debug)]
pub enum MasteringError {
    Dsp(Error),
    NoIntegratedLoudness,
}
impl From<Error> for MasteringError {
    fn from(error: Error) -> Self {
        Self::Dsp(error)
    }
}
impl std::fmt::Display for MasteringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp(e) => write!(f, "{e}"),
            Self::NoIntegratedLoudness => write!(
                f,
                "input is too short or falls below the integrated loudness gate"
            ),
        }
    }
}
impl std::error::Error for MasteringError {}

fn validate_audio(pcm: &[f32], rate: u32, channels: usize) -> Result<(), Error> {
    if !(8_000..=192_000).contains(&rate)
        || !(1..=2).contains(&channels)
        || !pcm.len().is_multiple_of(channels)
        || pcm.len() / channels > MAX_FRAMES
        || (pcm.len() / channels) as u64 * 48_000 > 28_800_000 * u64::from(rate)
        || pcm.iter().any(|x| !x.is_finite() || x.abs() > 16.)
    {
        return Err(Error::InvalidBuffer);
    }
    Ok(())
}

fn peak_dbtp(pcm: &[f32], channels: usize) -> Result<Option<f32>, Error> {
    let mut meter = TruePeakDetector::new(channels)?;
    for frame in pcm.chunks_exact(channels) {
        meter.push_frame(frame);
    }
    // Read the final 12 taps even if the render ends at a nonzero sample.
    for _ in 0..12 {
        meter.push_frame(&[0.; 2][..channels]);
    }
    Ok((meter.peak_linear() > 0.).then(|| meter.peak_dbtp()))
}

/// K-weighted loudness is measured at 48 kHz. Peaks are measured at the original rate.
pub fn measure(pcm: &[f32], rate: u32, channels: usize) -> Result<Measurements, Error> {
    validate_audio(pcm, rate, channels)?;
    let resampled;
    let loudness_pcm = if rate == 48_000 {
        pcm
    } else {
        resampled = resample_interleaved(
            pcm,
            channels,
            f64::from(rate),
            48_000.,
            ResampleQuality::SincFast,
        )?;
        &resampled
    };
    let frames = loudness_pcm.len() / channels;
    let mut meter = LoudnessMeter::new(48_000, channels, 1 + frames / 4800)?;
    meter.process(loudness_pcm)?;
    Ok(Measurements {
        integrated_lufs: meter.integrated_lufs(),
        true_peak_dbtp: peak_dbtp(pcm, channels)?,
    })
}

/// Analyze input, render with lookahead, remove the lookahead delay, then measure the actual output.
pub fn render_master(
    pcm: &[f32],
    rate: u32,
    channels: usize,
    options: Options,
) -> Result<Master, MasteringError> {
    options.validate()?;
    let input = measure(pcm, rate, channels)?;
    let lufs = input
        .integrated_lufs
        .ok_or(MasteringError::NoIntegratedLoudness)?;
    let level_gain_db = (options.target_lufs - lufs).clamp(-80., options.max_boost_db);
    let gain = 10f64.powf(level_gain_db / 20.) as f32;
    let mut limiter = LookaheadLimiter::default();
    limiter.set_lookahead_ms(options.lookahead_ms)?;
    limiter.set(options.peak_ceiling_dbtp, options.release_ms)?;
    limiter.prepare(ProcessSpec {
        sample_rate: f64::from(rate),
        max_block_frames: BLOCK,
        channels,
    })?;
    let latency = limiter.latency_frames();
    let mut output: Vec<_> = pcm.iter().map(|&x| x * gain).collect();
    output.resize(pcm.len() + latency * channels, 0.);
    let context = ProcessContext {
        events: &[],
        transport: Default::default(),
    };
    for block in output.chunks_mut(BLOCK * channels) {
        limiter.process(&mut AudioBuffer::new(block, channels)?, &context);
    }
    output.copy_within(latency * channels.., 0);
    output.truncate(pcm.len());

    let peak = peak_dbtp(&output, channels)?;
    let final_peak_trim_db = peak
        .filter(|&value| value > options.peak_ceiling_dbtp)
        .map_or(0., |value| options.peak_ceiling_dbtp - value - 0.02);
    if final_peak_trim_db != 0. {
        let trim = 10f32.powf(final_peak_trim_db / 20.);
        output.iter_mut().for_each(|x| *x *= trim);
    }
    let output_measurements = measure(&output, rate, channels)?;
    let met_target = output_measurements
        .integrated_lufs
        .is_some_and(|l| (l - options.target_lufs).abs() <= 0.25)
        && output_measurements
            .true_peak_dbtp
            .is_some_and(|p| p <= options.peak_ceiling_dbtp);
    Ok(Master {
        samples: output,
        report: Report {
            input,
            output: output_measurements,
            requested_lufs: options.target_lufs,
            requested_ceiling_dbtp: options.peak_ceiling_dbtp,
            level_gain_db,
            final_peak_trim_db,
            met_target,
            limiter_latency_frames: latency,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_loudness_and_preserves_stereo_length_at_44100() {
        let rate = 44_100;
        let mut pcm = vec![0.; rate as usize];
        for (i, frame) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let phase = std::f64::consts::TAU * 1000. * i as f64 / f64::from(rate);
            frame[0] = (0.055 * phase.sin()) as f32;
            frame[1] = -frame[0] * 0.25;
        }
        let options = Options::default();
        let render = render_master(&pcm, rate, 2, options).unwrap();
        assert_eq!(render.samples.len(), pcm.len());
        assert!(render.report.met_target, "{:?}", render.report);
        assert_eq!(render.samples[0], 0.);
        assert!(render
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .all(|&[l, r]| (l * -0.25 - r).abs() < 1e-6));
    }

    #[test]
    fn limiter_keeps_a_transient_below_the_meter_ceiling_and_rejects_silence() {
        let mut pcm: Vec<_> = (0..48_000)
            .map(|i| 0.04 * (i as f32 * 0.14).sin())
            .collect();
        pcm[48_000 - 1] = 1.;
        let options = Options {
            target_lufs: -11.,
            peak_ceiling_dbtp: -2.,
            ..Default::default()
        };
        let render = render_master(&pcm, 48_000, 1, options).unwrap();
        assert_eq!(render.samples.len(), 48_000);
        assert!(render.report.output.true_peak_dbtp.unwrap() <= -2.);
        assert!(render.samples[48_000 - 1] > 0.2);
        assert!(matches!(
            render_master(&[0.; 128], 48_000, 1, options),
            Err(MasteringError::NoIntegratedLoudness)
        ));
    }
}
