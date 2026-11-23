//! Offline, lossless storage for finite IEEE-float64 samples in a RIFF WAVE file.
use super::FileError;
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Debug)]
pub struct AudioFile64 {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f64>,
}

fn alignment(sample_rate: u32, channels: u16) -> Option<(u16, u32)> {
    if !(1..=64).contains(&channels) || sample_rate == 0 {
        return None;
    }
    let align = channels * 8;
    Some((align, sample_rate.checked_mul(u32::from(align))?))
}

/// Write ordinary little-endian RIFF/WAVE with IEEE float format tag 3.
/// Validates before creating a new destination and refuses an existing file.
pub fn write_wav64(
    path: impl AsRef<Path>,
    sample_rate: u32,
    channels: u16,
    samples: &[f64],
) -> Result<(), FileError> {
    let (align, byte_rate) =
        alignment(sample_rate, channels).ok_or("invalid float64 WAV layout")?;
    if !samples.len().is_multiple_of(channels as usize) || samples.iter().any(|s| !s.is_finite()) {
        return Err("invalid float64 WAV samples".into());
    }
    let bytes = samples
        .len()
        .checked_mul(8)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("float64 WAV exceeds RIFF size")?;
    let riff_size = bytes
        .checked_add(50)
        .ok_or("float64 WAV exceeds RIFF size")?;
    let frames = bytes / u32::from(align);
    let mut out = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&riff_size.to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&18u32.to_le_bytes())?;
    out.write_all(&3u16.to_le_bytes())?;
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&sample_rate.to_le_bytes())?;
    out.write_all(&byte_rate.to_le_bytes())?;
    out.write_all(&align.to_le_bytes())?;
    out.write_all(&64u16.to_le_bytes())?;
    out.write_all(&0u16.to_le_bytes())?;
    out.write_all(b"fact")?;
    out.write_all(&4u32.to_le_bytes())?;
    out.write_all(&frames.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        out.write_all(&sample.to_le_bytes())?;
    }
    out.flush()?;
    Ok(())
}

/// Read finite IEEE float64 RIFF/WAVE samples, subject to a decoded-byte limit.
pub fn read_wav64_with_limit(
    path: impl AsRef<Path>,
    max_pcm_bytes: usize,
) -> Result<AudioFile64, FileError> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let header = read::<12>(&mut reader)?;
    let end = 8 + u64::from(u32::from_le_bytes(header[4..8].try_into()?));
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" || !(12..=length).contains(&end) {
        return Err("invalid RIFF/WAVE header".into());
    }
    let mut format = None;
    let mut audio = None;
    let mut pos = 12;
    while pos < end {
        if end - pos < 8 {
            return Err("truncated WAVE chunk header".into());
        }
        let chunk = read::<8>(&mut reader)?;
        let len = u64::from(u32::from_le_bytes(chunk[4..].try_into()?));
        let start = pos + 8;
        pos = start + len + len % 2;
        if pos > end {
            return Err("truncated WAVE chunk".into());
        }
        match &chunk[..4] {
            b"fmt " if len >= 16 && format.is_none() => format = Some(read::<16>(&mut reader)?),
            b"data" if audio.is_none() => audio = Some((start, len)),
            b"fmt " | b"data" => return Err("ambiguous WAVE chunk".into()),
            _ => {}
        }
        // Other RIFF chunks carry metadata and do not affect sample data.
        reader.seek(SeekFrom::Start(pos))?;
    }
    let fmt = format.ok_or("missing WAVE format chunk")?;
    let (start, bytes) = audio.ok_or("missing WAVE audio chunk")?;
    let tag = u16::from_le_bytes(fmt[..2].try_into()?);
    let channels = u16::from_le_bytes(fmt[2..4].try_into()?);
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let (align, byte_rate) =
        alignment(sample_rate, channels).ok_or("invalid float64 WAV layout")?;
    if tag != 3
        || u16::from_le_bytes(fmt[14..].try_into()?) != 64
        || u32::from_le_bytes(fmt[8..12].try_into()?) != byte_rate
        || u16::from_le_bytes(fmt[12..14].try_into()?) != align
        || !bytes.is_multiple_of(u64::from(align))
        || bytes > max_pcm_bytes as u64
    {
        return Err("unsupported float64 WAV format or buffer size".into());
    }
    reader.seek(SeekFrom::Start(start))?;
    let mut samples = Vec::new();
    samples.try_reserve_exact(usize::try_from(bytes / 8)?)?;
    for _ in 0..bytes / 8 {
        let sample = f64::from_le_bytes(read::<8>(&mut reader)?);
        if !sample.is_finite() {
            return Err("nonfinite WAV sample".into());
        }
        samples.push(sample);
    }
    Ok(AudioFile64 {
        sample_rate,
        channels,
        samples,
    })
}

fn read<const N: usize>(input: &mut impl Read) -> std::io::Result<[u8; N]> {
    let mut data = [0; N];
    input.read_exact(&mut data)?;
    Ok(data)
}
