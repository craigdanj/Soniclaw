//! Conservative salvage of this recorder's interrupted 44-byte float WAV format. Never changes the source.
use crate::multitrack::RecordError;
use serde::Serialize;
use std::{io::Read, path::Path};
#[derive(Debug, Serialize)]
pub struct Report {
    pub sample_rate: u32,
    pub channels: u16,
    pub recovered_frames: u64,
    pub discarded_bytes: u64,
}
pub fn inspect(source: &Path) -> Result<Report, RecordError> {
    let mut f = std::fs::File::open(source)?;
    let bytes = f.metadata()?.len();
    if !(44..=2 * 1024 * 1024 * 1024).contains(&bytes) {
        return Err("unsupported interrupted WAV size".into());
    }
    let mut h = [0; 44];
    f.read_exact(&mut h)?;
    let word = |i| u16::from_le_bytes([h[i], h[i + 1]]);
    let long = |i| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    let channels = word(22);
    let rate = long(24);
    if &h[..4] != b"RIFF"
        || &h[8..16] != b"WAVEfmt "
        || long(16) != 16
        || word(20) != 3
        || !(1..=8).contains(&channels)
        || !(8000..=192000).contains(&rate)
        || word(32) != channels * 4
        || word(34) != 32
        || &h[36..40] != b"data"
        || long(28) != rate * channels as u32 * 4
    {
        return Err("not a supported Soniclaw float recording header".into());
    }
    let frame = channels as u64 * 4;
    Ok(Report {
        sample_rate: rate,
        channels,
        recovered_frames: (bytes - 44) / frame,
        discarded_bytes: (bytes - 44) % frame,
    })
}
/// Copy finite, whole physical frames to a new standard float WAV. Header lengths from the interrupted source are ignored.
pub fn recover(source: &Path, destination: &Path) -> Result<Report, RecordError> {
    let report = inspect(source)?;
    if report.recovered_frames == 0 {
        return Err("interrupted recording has no complete frames".into());
    }
    let mut input = std::fs::File::open(source)?;
    let mut header = [0; 44];
    input.read_exact(&mut header)?;
    let output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(destination)?;
    let result = (|| -> Result<(), RecordError> {
        let mut writer = hound::WavWriter::new(
            std::io::BufWriter::new(output),
            hound::WavSpec {
                sample_rate: report.sample_rate,
                channels: report.channels,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        let mut remaining = report.recovered_frames * report.channels as u64;
        let mut bytes = [0; 65536];
        while remaining > 0 {
            let count = remaining.min(16384) as usize;
            input.read_exact(&mut bytes[..count * 4])?;
            for b in bytes[..count * 4].as_chunks::<4>().0 {
                let value = f32::from_le_bytes(*b);
                if !value.is_finite() || value.abs() > 1e6 {
                    return Err("interrupted recording contains invalid PCM".into());
                }
                writer.write_sample(value)?;
            }
            remaining -= count as u64;
        }
        writer.finalize()?;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(destination)?
            .sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(destination);
        return Err(e);
    }
    Ok(report)
}
