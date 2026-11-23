//! Offline studio mix/stem export. Exclusive output directory, manifest last, bounded streaming buffers.
use crate::{
    jobs::Context,
    project::{Project, ProjectError},
    *,
};
use soniclaw_dsp::advanced::analysis::LoudnessMeter;
use soniclaw_formats::{AudioFormat, Encoding, FormatManager};
use std::path::{Path, PathBuf};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Format {
    Wav,
    Aiff,
    Flac,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Depth {
    Float32,
    Pcm16,
    Pcm24,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Normalize {
    None,
    Peak(f32),
    Lufs(f64),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema: u32,
    #[serde(default)]
    pub output_rate: Option<u32>,
    pub mix: bool,
    pub stems: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routed_stems: Vec<crate::delivery::Source>,
    pub start_frame: u64,
    pub length_frames: u64,
    pub tail_frames: u64,
    pub format: Format,
    pub depth: Depth,
    pub normalize: Normalize,
    pub peak_ceiling: f32,
    pub dither: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            output_rate: None,
            mix: true,
            stems: vec![],
            routed_stems: vec![],
            start_frame: 0,
            length_frames: 192000,
            tail_frames: 48000,
            format: Format::Wav,
            depth: Depth::Float32,
            normalize: Normalize::None,
            peak_ceiling: 0.99,
            dither: false,
        }
    }
}
impl Settings {
    pub fn validate(&self, p: &Project) -> Result<(), ProjectError> {
        p.validate()?;
        let rate = self.output_rate.unwrap_or(p.document.sample_rate);
        if !(8000..=192000).contains(&rate) {
            return Err("output rate must be 8..192 kHz".into());
        }
        let mut ids = BTreeSet::new();
        let frames = self
            .length_frames
            .checked_add(self.tail_frames)
            .ok_or("export range overflow")?;
        if (frames as f64 * rate as f64 / p.document.sample_rate as f64).round()
            > u32::MAX as f64 / 2.
        {
            return Err("converted output exceeds RIFF bound".into());
        }
        if self.schema != 1
            || !self.mix && self.stems.is_empty() && self.routed_stems.is_empty()
            || self.stems.len() + self.routed_stems.len() + usize::from(self.mix) > 32
            || self
                .stems
                .iter()
                .any(|id| !ids.insert(id) || !p.document.tracks.iter().any(|t| &t.id == id))
            || self.length_frames == 0
            || frames > u32::MAX as u64 / 2
            || self
                .start_frame
                .checked_add(frames)
                .is_none_or(|v| v > p.document.sample_rate as u64 * 14400)
            || p.document.timeline.loop_range.is_some()
            || p.document
                .tracks
                .iter()
                .any(|t| t.input_gain != 0. || t.midi_input)
            || !self.peak_ceiling.is_finite()
            || !(0.01..=1.).contains(&self.peak_ceiling)
            || self.format == Format::Flac && self.depth == Depth::Float32
            || self.dither && self.depth == Depth::Float32
        {
            return Err("invalid export selection/range/live input/loop/codec controls".into());
        }
        for (i, source) in self.routed_stems.iter().enumerate() {
            if self.routed_stems[..i].contains(source)
                || match source {
                    crate::delivery::Source::TrackPost(id) => {
                        !p.document.tracks.iter().any(|t| &t.id == id)
                    }
                    crate::delivery::Source::Bus(id) => {
                        !p.document.mix_routing.buses.iter().any(|b| &b.id == id)
                    }
                    crate::delivery::Source::Aux(i) => *i >= p.document.auxiliaries.len(),
                }
            {
                return Err("invalid or duplicate routed export source".into());
            }
        }
        match self.normalize {
            Normalize::Peak(v) if !v.is_finite() || !(0.01..=1.).contains(&v) => {
                return Err("peak target must be 0.01..1".into())
            }
            Normalize::Lufs(v)
                if !v.is_finite() || !(-60.0..=-5.).contains(&v) || rate != 48000 =>
            {
                return Err("LUFS normalization requires 48 kHz output and target -60..-5".into())
            }
            _ => {}
        }
        Ok(())
    }
    pub fn to_json(&self, p: &Project) -> Result<String, ProjectError> {
        self.validate(p)?;
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str, p: &Project) -> Result<Self, ProjectError> {
        if s.len() > 16384 {
            return Err("export settings exceed 16 KiB".into());
        }
        let v: Self = serde_json::from_str(s)?;
        v.validate(p)?;
        Ok(v)
    }
}
#[derive(Debug, Serialize)]
pub struct FileReport {
    pub file: String,
    pub role: String,
    pub frames: u64,
    pub source_peak: f32,
    pub source_lufs: Option<f64>,
    pub gain: f64,
    pub output_sample_peak: f32,
    pub estimated_output_lufs: Option<f64>,
    pub ceiling_limited: bool,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub project_id: String,
    pub settings: Settings,
    pub sample_rate: u32,
    pub files: Vec<FileReport>,
}
fn check(c: &Context) -> Result<(), ProjectError> {
    if c.cancelled() {
        Err("export cancelled".into())
    } else {
        Ok(())
    }
}
fn fingerprint(p: &Project, c: &Context) -> Result<String, ProjectError> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(p.document.to_json()?);
    h.update(serde_json::to_vec(&p.native_states)?);
    for (id, a) in &p.assets {
        h.update(id);
        for s in a.samples.iter() {
            h.update(s.to_bits().to_le_bytes());
        }
    }
    for (id, path) in &p.disk {
        h.update(id);
        let mut f = std::fs::File::open(path)?;
        let mut b = [0; 65536];
        loop {
            check(c)?;
            let n = std::io::Read::read(&mut f, &mut b)?;
            if n == 0 {
                break;
            }
            h.update(&b[..n]);
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
fn master(
    p: &Project,
    s: &Settings,
    factory: &mut impl Factory,
    path: &Path,
    c: &Context,
) -> Result<(), ProjectError> {
    let mut session = p.document.compile_offline(&p.assets, &p.disk, factory)?;
    let latency = session.latency_frames() as u64;
    let end = s.start_frame + s.length_frames + s.tail_frames;
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: p.document.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut data = vec![0.; p.document.max_block_frames * 2];
    let mut clock = 0;
    while clock < end + latency {
        check(c)?;
        let n = ((end + latency - clock) as usize).min(p.document.max_block_frames);
        data[..n * 2].fill(0.);
        session.render_block(
            &mut AudioBuffer::new(&mut data[..n * 2], 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: clock,
                    playing: true,
                    ..Default::default()
                },
            },
        )?;
        for f in 0..n {
            let pos = clock + f as u64;
            if pos >= s.start_frame + latency && pos < end + latency {
                for v in &data[f * 2..f * 2 + 2] {
                    w.write_sample(*v)?;
                }
            }
        }
        clock += n as u64;
        c.progress(clock, end + latency);
    }
    w.finalize()?;
    Ok(())
}
fn analyse(path: &Path, rate: u32, c: &Context) -> Result<(f32, Option<f64>), ProjectError> {
    let mut r = hound::WavReader::open(path)?;
    let mut peak = 0f32;
    let mut meter = if rate == 48000 {
        Some(LoudnessMeter::new(rate, 2, 144000)?)
    } else {
        None
    };
    let mut block = Vec::with_capacity(8192);
    for v in r.samples::<f32>() {
        let v = v?;
        if !v.is_finite() {
            return Err("nonfinite export PCM".into());
        }
        peak = peak.max(v.abs());
        block.push(v);
        if block.len() == 8192 {
            check(c)?;
            if let Some(m) = &mut meter {
                m.process(&block)?;
            }
            block.clear();
        }
    }
    if let Some(m) = &mut meter {
        m.process(&block)?;
    }
    Ok((
        peak,
        meter.and_then(|m| m.integrated_lufs().filter(|v| v.is_finite())),
    ))
}
fn random(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed as f32 / u32::MAX as f32
}
/// Runs on the caller's offline/control worker. Factory must be owned on this thread.
pub fn render(
    p: &Project,
    s: Settings,
    directory: &Path,
    factory: &mut impl Factory,
    c: &Context,
) -> Result<Report, ProjectError> {
    s.validate(p)?;
    check(c)?;
    let rate = s.output_rate.unwrap_or(p.document.sample_rate);
    let expected_frames = ((s.length_frames + s.tail_frames) as f64 * rate as f64
        / p.document.sample_rate as f64)
        .round() as u64;
    let manager = FormatManager::native_default()?;
    let format = match s.format {
        Format::Wav => AudioFormat::Wav,
        Format::Aiff => AudioFormat::Aiff,
        Format::Flac => AudioFormat::Flac,
    };
    let encoding = match s.depth {
        Depth::Float32 => Encoding::Float32,
        Depth::Pcm16 => Encoding::Pcm16,
        Depth::Pcm24 => Encoding::Pcm24,
    };
    if !manager.supports(format, encoding, rate, 2) {
        return Err("installed codec backend does not support export choice".into());
    }
    let before = fingerprint(p, c)?;
    std::fs::create_dir(directory)?;
    let result = (|| -> Result<Report, ProjectError> {
        let mut inputs: Vec<(String, PathBuf)> = vec![];
        if !s.routed_stems.is_empty() {
            inputs.extend(routed(p, &s, factory, directory, c)?);
        }
        if s.mix && s.routed_stems.is_empty() {
            let path = directory.join("working-mix.wav");
            master(p, &s, factory, &path, c)?;
            inputs.push(("mix".into(), path));
        }
        for (i, id) in s.stems.iter().enumerate() {
            check(c)?;
            let temp = directory.join(format!("working-stem-{i}"));
            let monitor = freeze::Monitor::default().with_cancel(c.cancel_flag());
            let frozen = freeze::render_monitored(
                &p.document,
                id,
                &p.assets,
                &p.disk,
                factory,
                &temp,
                s.start_frame,
                s.length_frames,
                s.tail_frames,
                &monitor,
            )?;
            inputs.push((
                format!("stem-{i}-{}", id),
                frozen.directory.join("stem.wav"),
            ));
        }
        let mut files = vec![];
        for (i, (role, input)) in inputs.iter().enumerate() {
            check(c)?;
            let converted = directory.join(format!("working-converted-{i}.wav"));
            let converted_input = if rate != p.document.sample_rate {
                crate::export_convert::convert(input, &converted, rate, c)?;
                Some(converted.clone())
            } else {
                None
            };
            let encoding_input = converted_input.as_deref().unwrap_or(input);
            let (peak, lufs) = analyse(encoding_input, rate, c)?;
            let desired = match s.normalize {
                Normalize::None => 1.,
                Normalize::Peak(v) => {
                    if peak > 0. {
                        v as f64 / peak as f64
                    } else {
                        1.
                    }
                }
                Normalize::Lufs(v) => {
                    10f64.powf((v - lufs.ok_or("LUFS unavailable for silent/short export")?) / 20.)
                }
            };
            let gain = if matches!(s.normalize, Normalize::None) {
                desired
            } else {
                desired.min(if peak > 0. {
                    s.peak_ceiling as f64 / peak as f64
                } else {
                    desired
                })
            };
            let extension = match s.format {
                Format::Wav => "wav",
                Format::Aiff => "aiff",
                Format::Flac => "flac",
            };
            let file = format!("{role}.{extension}");
            let mut writer = manager.create(&directory.join(&file), format, encoding, rate, 2)?;
            writer.set_metadata("title", &p.name)?;
            let mut reader = hound::WavReader::open(encoding_input)?;
            let mut buffer = Vec::with_capacity(8192);
            let mut seed = 0x534f4e49u32;
            let lsb = match s.depth {
                Depth::Pcm16 => 1. / 32768.,
                Depth::Pcm24 => 1. / 8388608.,
                Depth::Float32 => 0.,
            };
            let mut output_peak = 0f32;
            for x in reader.samples::<f32>() {
                let mut v = (x? as f64 * gain) as f32;
                if s.dither {
                    v += (random(&mut seed) - random(&mut seed)) * lsb;
                }
                if s.depth != Depth::Float32 {
                    v = v.clamp(-1., 1. - lsb);
                }
                output_peak = output_peak.max(v.abs());
                buffer.push(v);
                if buffer.len() == 8192 {
                    check(c)?;
                    writer.write_frames(&buffer)?;
                    buffer.clear();
                }
            }
            writer.write_frames(&buffer)?;
            let frames = writer.finish()?;
            if converted_input.is_some() {
                std::fs::remove_file(converted)?;
            }
            if frames != expected_frames {
                return Err("encoded export length mismatch".into());
            }
            files.push(FileReport {
                file,
                role: role.clone(),
                frames,
                source_peak: peak,
                source_lufs: lufs,
                gain,
                output_sample_peak: output_peak,
                estimated_output_lufs: lufs.map(|v| v + 20. * gain.log10()),
                ceiling_limited: gain < desired,
            });
            c.progress((i + 1) as u64, inputs.len() as u64);
        }
        if fingerprint(p, c)? != before {
            return Err("project media changed during export".into());
        }
        check(c)?;
        for (_, input) in inputs {
            if input.file_name().is_some_and(|n| n == "stem.wav") {
                std::fs::remove_dir_all(input.parent().unwrap())?;
            } else {
                std::fs::remove_file(input)?;
            }
        }
        let report = Report {
            schema: 1,
            project_id: p.project_id.clone(),
            settings: s,
            sample_rate: rate,
            files,
        };
        std::fs::write(
            directory.join("export.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result
}
/// One graph pass preserves sidechain detectors, nested routes and effect histories for every selected tap.
fn routed(
    p: &Project,
    s: &Settings,
    factory: &mut impl Factory,
    directory: &Path,
    c: &Context,
) -> Result<Vec<(String, PathBuf)>, ProjectError> {
    let mut session = p.document.compile_offline(&p.assets, &p.disk, factory)?;
    let mut selected = Vec::new();
    let mut result = Vec::new();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: p.document.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    if s.mix {
        let path = directory.join("working-mix.wav");
        selected.push((
            None,
            session.latency_frames() as u64,
            hound::WavWriter::create(&path, spec)?,
        ));
        result.push(("mix".into(), path));
    }
    for (i, source) in s.routed_stems.iter().enumerate() {
        let tap = session
            .routed_taps()
            .iter()
            .find(|t| &t.source == source)
            .ok_or("missing routed tap")?
            .clone();
        tap.enable();
        let role = match source {
            crate::delivery::Source::TrackPost(id) => format!("route-{i}-track-{id}"),
            crate::delivery::Source::Bus(id) => format!("route-{i}-bus-{id}"),
            crate::delivery::Source::Aux(index) => format!("route-{i}-aux-{index}"),
        };
        let path = directory.join(format!("working-route-{i}.wav"));
        selected.push((
            Some(tap.bank),
            tap.latency_frames as u64,
            hound::WavWriter::create(&path, spec)?,
        ));
        result.push((role, path));
    }
    let max_latency = selected.iter().map(|(_, l, _)| *l).max().unwrap_or(0);
    let end = s.start_frame + s.length_frames + s.tail_frames;
    let mut clock = 0u64;
    let mut data = vec![0.; p.document.max_block_frames * 2];
    while clock < end + max_latency {
        check(c)?;
        let n = ((end + max_latency - clock) as usize).min(p.document.max_block_frames);
        data[..n * 2].fill(0.);
        session.render_block(
            &mut AudioBuffer::new(&mut data[..n * 2], 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: clock,
                    playing: true,
                    ..Default::default()
                },
            },
        )?;
        for (bank, latency, writer) in &mut selected {
            for f in 0..n {
                let pos = clock + f as u64;
                if pos >= s.start_frame + *latency && pos < end + *latency {
                    for ch in 0..2 {
                        writer.write_sample(
                            bank.as_ref().map_or(data[f * 2 + ch], |b| b.sample(f, ch)),
                        )?;
                    }
                }
            }
        }
        clock += n as u64;
        c.progress(clock, end + max_latency);
    }
    for (_, _, writer) in selected {
        writer.finalize()?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_reject_live_loop_duplicate_codec_and_future_schema() {
        let (mut d, a) = crate::tests::fixture();
        d.timeline.loop_range = None;
        let p = Project::new("Export".into(), d, a, Default::default()).unwrap();
        let s = Settings::default();
        assert!(s.validate(&p).is_ok());
        let json = s.to_json(&p).unwrap();
        assert!(Settings::from_json(&json, &p).is_ok());
        let mut bad = s.clone();
        bad.stems = vec!["audio".into(), "audio".into()];
        assert!(bad.validate(&p).is_err());
        bad = s.clone();
        bad.format = Format::Flac;
        assert!(bad.validate(&p).is_err());
        bad = s.clone();
        bad.schema = 2;
        assert!(bad.validate(&p).is_err());
        let mut live = p.clone();
        live.document.tracks[0].input_gain = 1.;
        assert!(s.validate(&live).is_err());
        live = p;
        live.document.timeline.loop_range = Some([0., 4.]);
        assert!(s.validate(&live).is_err());
    }
}
