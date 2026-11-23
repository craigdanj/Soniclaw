//! Bounded offline sinc conversion for export staging files. Never runs on a callback.
use crate::{jobs::Context, project::ProjectError};
use soniclaw_dsp::resample::ResampleQuality;
use std::path::Path;
/// Input must be a stereo float32 staging WAV. Output path must be absent and caller-owned.
pub fn convert(
    input: &Path,
    output: &Path,
    target_rate: u32,
    c: &Context,
) -> Result<u64, ProjectError> {
    if !(8000..=192000).contains(&target_rate) || output.exists() {
        return Err("invalid conversion rate or existing output".into());
    }
    let mut reader = hound::WavReader::open(input)?;
    let spec = reader.spec();
    if spec.channels != 2
        || spec.sample_format != hound::SampleFormat::Float
        || spec.bits_per_sample != 32
        || !(8000..=192000).contains(&spec.sample_rate)
    {
        return Err("conversion requires stereo float32 staging WAV at 8..192 kHz".into());
    }
    let source_frames = reader.duration() as u64;
    let frames =
        (source_frames as f64 * target_rate as f64 / spec.sample_rate as f64).round() as u64;
    if frames == 0 || frames > u32::MAX as u64 / 2 {
        return Err("converted file exceeds RIFF bound or is empty".into());
    }
    let quality = ResampleQuality::SincBest;
    let increment = spec.sample_rate as f64 / target_rate as f64;
    let radius = quality.radius(increment) as u64;
    let mut source = vec![0.; ((4096. * increment).ceil() as usize + 2 * radius as usize + 4) * 2];
    let mut output_block = vec![0.; 8192];
    let mut writer = hound::WavWriter::create(
        output,
        hound::WavSpec {
            channels: 2,
            sample_rate: target_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let result = (|| -> Result<u64, ProjectError> {
        let mut written = 0;
        while written < frames {
            if c.cancelled() {
                return Err("sample conversion cancelled".into());
            }
            let n = (frames - written).min(4096) as usize;
            let first = written as f64 * increment;
            let last = (written + n as u64 - 1) as f64 * increment;
            let begin = (first.floor() as u64).saturating_sub(radius);
            let end = ((last.ceil() as u64) + radius + 1).min(source_frames);
            let count = (end - begin) as usize;
            if count * 2 > source.len() {
                return Err("conversion working set overflow".into());
            }
            reader.seek(begin as u32)?;
            let mut samples = reader.samples::<f32>();
            for s in &mut source[..count * 2] {
                *s = samples.next().ok_or("short staging input")??;
                if !s.is_finite() {
                    return Err("nonfinite staging input".into());
                }
            }
            for frame in 0..n {
                let position = (written + frame as u64) as f64 * increment;
                for channel in 0..2 {
                    output_block[frame * 2 + channel] =
                        quality.interpolate(position, increment, |index| {
                            if index < 0 {
                                return 0.;
                            }
                            let index = index as u64;
                            if index < begin || index >= end {
                                0.
                            } else {
                                source[((index - begin) as usize) * 2 + channel]
                            }
                        });
                }
            }
            for &s in &output_block[..n * 2] {
                writer.write_sample(s)?;
            }
            written += n as u64;
            c.progress(written, frames);
        }
        Ok(written)
    })();
    match result {
        Ok(n) => {
            writer.finalize()?;
            Ok(n)
        }
        Err(e) => {
            drop(writer);
            let _ = std::fs::remove_file(output);
            Err(e)
        }
    }
}
