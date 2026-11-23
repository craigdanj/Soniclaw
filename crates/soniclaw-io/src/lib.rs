//! Blocking, allocating file APIs. Never call these on an audio thread.
mod float_wav;
pub use float_wav::{read_wav64_with_limit, write_wav64, AudioFile64};
use std::path::Path;
pub type FileError = Box<dyn std::error::Error>;
#[derive(Debug)]
pub struct AudioFile {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}
impl AudioFile {
    /// Allocating, control-thread sample-rate conversion with selectable quality.
    pub fn resampled(
        &self,
        rate: u32,
        quality: soniclaw_dsp::resample::ResampleQuality,
    ) -> Result<Self, FileError> {
        Ok(Self {
            sample_rate: rate,
            channels: self.channels,
            samples: soniclaw_dsp::resample::resample_interleaved(
                &self.samples,
                self.channels as usize,
                self.sample_rate as f64,
                rate as f64,
                quality,
            )?,
        })
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
/// Read PCM up to 32 bit or IEEE float32 WAV. No resampling is performed.
pub fn read_wav(path: impl AsRef<Path>) -> Result<AudioFile, FileError> {
    read_wav_with_limit(path, 256 * 1024 * 1024)
}
/// Reject oversized decoded PCM before allocating. Limit counts f32 PCM bytes.
pub fn read_wav_with_limit(
    path: impl AsRef<Path>,
    max_pcm_bytes: usize,
) -> Result<AudioFile, FileError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err("invalid WAV configuration".into());
    }
    let bytes = (reader.duration() as usize)
        .checked_mul(spec.channels as usize)
        .and_then(|x| x.checked_mul(4))
        .ok_or("PCM length overflow")?;
    if bytes > max_pcm_bytes {
        return Err("decoded PCM exceeds memory budget".into());
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map_err(Into::into))
            .collect::<Result<_, FileError>>()?,
        hound::SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err("unsupported PCM width".into());
            }
            let scale = 2.0_f64.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|x| (x as f64 / scale) as f32).map_err(Into::into))
                .collect::<Result<_, FileError>>()?
        }
    };
    if !samples.len().is_multiple_of(spec.channels as usize)
        || samples.iter().any(|s| !s.is_finite())
    {
        return Err("WAV contains partial frames or non-finite audio".into());
    }
    Ok(AudioFile {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        samples,
    })
}
/// PCM16 output, hard saturation at [-1,1]. Rejects NaN/infinity before opening.
pub fn write_wav(
    path: impl AsRef<Path>,
    sample_rate: u32,
    channels: u16,
    data: &[f32],
) -> Result<(), FileError> {
    if sample_rate == 0
        || channels == 0
        || !data.len().is_multiple_of(channels as usize)
        || data.iter().any(|s| !s.is_finite())
    {
        return Err("invalid audio for WAV output".into());
    }
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in data {
        writer.write_sample((s.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stereo_round_trip_and_saturation() {
        let dir = std::env::temp_dir().join(format!("soniclaw-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.wav");
        let input = [0.0, 0.25, -0.5, 2.0];
        write_wav(&path, 48000, 2, &input).unwrap();
        let out = read_wav(&path).unwrap();
        assert_eq!(out.frames(), 2);
        assert_eq!(out.channels, 2);
        assert!((out.samples[1] - 0.25).abs() < 0.0001);
        assert!((out.samples[3] - 1.0).abs() < 0.0001);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
