//! Explicit-directory media browsing, bounded worker decoding, MIDI import and portable stock preset folders.
use crate::{
    project::{Project, ProjectError},
    *,
};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Kind {
    Folder,
    Preset,
    Audio,
    Midi,
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub bytes: u64,
}
pub fn scan(directory: &Path, query: &str) -> Result<Vec<Entry>, ProjectError> {
    if query.len() > 256 {
        return Err("media query too long".into());
    }
    let directory = directory.canonicalize()?;
    let query = query.to_lowercase();
    let mut result = vec![];
    let mut count = 0;
    for e in std::fs::read_dir(directory)? {
        count += 1;
        if count > 4096 {
            return Err("media directory exceeds 4096 entries; choose a narrower directory".into());
        }
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !name.to_lowercase().contains(&query) {
            continue;
        }
        let ty = e.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let path = e.path();
        let kind = if ty.is_dir() {
            if path.join("project.json").is_file() {
                Kind::Preset
            } else {
                Kind::Folder
            }
        } else if ty.is_file() {
            match path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "wav" | "aif" | "aiff" | "flac" => Kind::Audio,
                "mid" | "midi" => Kind::Midi,
                _ => continue,
            }
        } else {
            continue;
        };
        result.push(Entry {
            name,
            path,
            kind,
            bytes: e.metadata()?.len(),
        });
    }
    result.sort_by_key(|a| a.name.to_lowercase());
    Ok(result)
}
/// Decode at most 128 MiB source PCM, resample off callback, return project-rate mono/stereo.
pub fn decode(path: &Path, rate: u32) -> Result<soniclaw_clips::AudioAsset, ProjectError> {
    if !(8000..=192000).contains(&rate) || std::fs::metadata(path)?.len() > 1024 * 1024 * 1024 {
        return Err("invalid media rate/file size".into());
    }
    let (channels, source_rate, data) = if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
    {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if !(1..=2).contains(&spec.channels) || reader.len() as usize > 32 * 1024 * 1024 {
            return Err("sample must fit 128 MiB mono/stereo budget".into());
        }
        let data = if spec.sample_format == hound::SampleFormat::Float {
            reader
                .into_samples::<f32>()
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let scale = 2f64.powi(spec.bits_per_sample as i32 - 1);
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|s| (s as f64 / scale) as f32))
                .collect::<Result<Vec<_>, _>>()?
        };
        (spec.channels, spec.sample_rate, data)
    } else {
        let file =
            soniclaw_formats::FormatManager::native_default()?.read_all(path, 128 * 1024 * 1024)?;
        (file.channels, file.sample_rate, file.samples)
    };
    if !(1..=2).contains(&channels)
        || data.is_empty()
        || data.iter().any(|s| !s.is_finite())
        || source_rate == 0
    {
        return Err("invalid decoded sample data".into());
    }
    if data.len() as f64 * rate as f64 / source_rate as f64 > 32. * 1024. * 1024. {
        return Err("resampled media exceeds sample budget".into());
    }
    let samples = if rate == source_rate {
        data
    } else {
        soniclaw_dsp::resample::resample_interleaved(
            &data,
            channels as usize,
            source_rate as f64,
            rate as f64,
            soniclaw_dsp::resample::ResampleQuality::SincBest,
        )?
    };
    Ok(soniclaw_clips::AudioAsset {
        sample_rate: rate,
        channels,
        samples: Arc::from(samples),
    })
}
pub fn place_audio(
    p: &mut Project,
    track: &str,
    asset_id: &str,
    clip_id: &str,
    asset: soniclaw_clips::AudioAsset,
    start_beat: f64,
) -> Result<(), ProjectError> {
    if !valid_id(asset_id) || p.assets.contains_key(asset_id) {
        return Err("audio asset identity collision".into());
    }
    let mut next = p.clone();
    let i = next
        .document
        .tracks
        .iter()
        .position(|t| t.id == track)
        .ok_or("audio destination missing")?;
    let frames = asset.frames();
    let start = TempoMap::compile(&next.document.timeline)?
        .sample_at(start_beat, next.document.sample_rate as f64)?;
    next.assets.insert(asset_id.into(), asset);
    next.document.tracks[i].audio.push(AudioClip {
        id: clip_id.into(),
        asset: asset_id.into(),
        track: i as u16,
        start_frame: start,
        length_frames: frames,
        source_offset: 0,
        source_length: frames,
        looping: false,
        gain: 1.,
        fade_in: 0,
        fade_out: 0,
        enabled: true,
        edits: None,
    });
    next.validate()?;
    *p = next;
    Ok(())
}
/// Import MIDI 1 notes/controllers by musical ticks. Optional file tempo overlays only its span.
pub fn import_midi(
    p: &mut Project,
    path: &Path,
    track: &str,
    start_beat: f64,
    file_tempo: bool,
) -> Result<String, ProjectError> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("MIDI import exceeds 16 MiB".into());
    }
    let smf = soniclaw_midi::file::MidiClip::read(&bytes)?;
    if smf.events.is_empty() || smf.events.len() > 65536 {
        return Err("MIDI import event count unsupported".into());
    }
    let end_beat = start_beat
        + smf.events.last().ok_or("MIDI empty")?.tick as f64 / smf.ppqn as f64
        + 1. / smf.ppqn as f64;
    let mut next = p.clone();
    if file_tempo {
        if next
            .document
            .timeline
            .tempo
            .iter()
            .any(|p| p.transition != soniclaw_transport::Transition::Step)
        {
            return Err("file tempo overlay requires a stepped project tempo map".into());
        }
        let old = TempoMap::compile(&next.document.timeline)?;
        let restore = old.position(end_beat)?.bpm;
        let mut timeline = next.document.timeline.clone();
        timeline
            .tempo
            .retain(|point| point.beat < start_beat || point.beat > end_beat);
        timeline.tempo.push(soniclaw_transport::TempoPoint {
            beat: start_beat,
            bpm: 120.,
            transition: soniclaw_transport::Transition::Step,
        });
        for tempo in &smf.tempos {
            timeline.tempo.push(soniclaw_transport::TempoPoint {
                beat: start_beat + tempo.tick as f64 / smf.ppqn as f64,
                bpm: 60_000_000. / tempo.micros_per_quarter as f64,
                transition: soniclaw_transport::Transition::Step,
            });
        }
        timeline.tempo.push(soniclaw_transport::TempoPoint {
            beat: end_beat,
            bpm: restore,
            transition: soniclaw_transport::Transition::Step,
        });
        timeline.tempo.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        timeline.tempo.reverse();
        timeline.tempo.dedup_by(|a, b| a.beat == b.beat);
        timeline.tempo.reverse();
        crate::arrangement::change_timeline(
            &mut next,
            timeline,
            crate::arrangement::AudioAnchor::Seconds,
        )?;
    }
    let mut timeline = next.document.timeline.clone();
    timeline.loop_range = None;
    let map = TempoMap::compile(&timeline)?;
    let rate = next.document.sample_rate as f64;
    let start = map.sample_at(start_beat, rate)?;
    let end = map.sample_at(end_beat, rate)?.max(start + 1);
    let seconds = ((end - start) as f64 / rate).ceil().max(1.) as u32;
    if seconds > 14400 {
        return Err("MIDI import exceeds four hours".into());
    }
    let id = workspace::identity("midi")?;
    let mut events = vec![];
    for e in smf.events {
        let sample = map
            .sample_at(start_beat + e.tick as f64 / smf.ppqn as f64, rate)?
            .min(end - 1);
        let message = midi_recording::Message::from_core(e.message)
            .ok_or("unsupported MIDI import message")?;
        events.push(midi_recording::Captured {
            sample,
            message,
            source_sample: None,
            chased: false,
        });
    }
    let take = midi_recording::Take {
        schema: 3,
        project_id: next.project_id.clone(),
        settings: midi_recording::Settings {
            track: track.into(),
            clip: id.clone(),
            start_beat,
            max_seconds: seconds,
            max_events: events.len().max(128),
            ..Default::default()
        },
        timeline,
        sample_rate: next.document.sample_rate,
        start_sample: start,
        end_sample: end,
        lost_events: 0,
        unsupported_messages: 0,
        discontinuities: 0,
        input_dropped: 0,
        events,
        interrupted: false,
        recovered_from_checkpoint: false,
        cycles: vec![],
    };
    let i = next
        .document
        .tracks
        .iter()
        .position(|t| t.id == track && t.instrument.is_some())
        .ok_or("select an instrument track for MIDI")?;
    let clip = take.note_clip(i as u16)?;
    if clip.notes.is_empty() && clip.controls.is_empty() {
        return Err("MIDI file contains no editable performance".into());
    }
    next.document.tracks[i].midi.push(clip);
    next.validate()?;
    *p = next;
    Ok(id)
}
#[derive(Clone)]
pub struct Preset {
    pub name: String,
    pub instrument: Option<Descriptor>,
    pub inserts: Vec<Descriptor>,
    pub values: BTreeMap<String, f32>,
    pub assets: Assets,
}
/// Save one selected stock instrument or insert in a self-contained one-track portable carrier project.
pub fn save_preset(
    p: &Project,
    descriptor: &str,
    name: &str,
    directory: &Path,
) -> Result<(), ProjectError> {
    let d = p
        .document
        .tracks
        .iter()
        .flat_map(|t| t.instrument.iter().chain(&t.inserts))
        .find(|d| d.id == descriptor)
        .ok_or("preset processor missing")?;
    if !d.kind.starts_with("stock.") {
        return Err("stock preset folders accept stock processors only".into());
    }
    let mut carrier = workspace::blank(
        name.into(),
        p.document.sample_rate,
        p.document.max_block_frames,
    )?;
    if d.kind == "stock.fx" {
        carrier.document.tracks[0].inserts.push(d.clone());
    } else {
        carrier.document.tracks[0].instrument = Some(d.clone());
    }
    let prefix = format!("{}.", d.id);
    carrier.document.parameter_values = p
        .document
        .parameter_values
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(key, v)| (key.clone(), *v))
        .collect();
    if d.kind == "stock.sampler" {
        let id = d.state["asset"]
            .as_str()
            .ok_or("preset sample identity missing")?;
        carrier.assets.insert(
            id.into(),
            p.assets.get(id).ok_or("preset sample missing")?.clone(),
        );
    }
    carrier.save_new(directory)
}
pub fn load_preset(directory: &Path, rate: u32) -> Result<Preset, ProjectError> {
    if !(8000..=192000).contains(&rate) {
        return Err("invalid preset target rate".into());
    }
    let carrier = Project::open(directory)?;
    if carrier.document.tracks.len() != 1 || !carrier.takes.is_empty() || !carrier.disk.is_empty() {
        return Err("not a one-track stock preset carrier".into());
    }
    let t = &carrier.document.tracks[0];
    let descriptors = t.instrument.iter().chain(&t.inserts);
    if descriptors.clone().count() != 1
        || descriptors.clone().any(|d| !d.kind.starts_with("stock."))
        || !t.audio.is_empty()
        || !t.midi.is_empty()
        || !t.streamed.is_empty()
    {
        return Err("invalid stock preset contents".into());
    }
    let mut assets = carrier.assets;
    for a in assets.values_mut() {
        if a.samples.len() as f64 * rate as f64 / a.sample_rate as f64 > 32. * 1024. * 1024. {
            return Err("resampled preset sample exceeds 128 MiB".into());
        }
        if a.sample_rate != rate {
            let data = soniclaw_dsp::resample::resample_interleaved(
                &a.samples,
                a.channels as usize,
                a.sample_rate as f64,
                rate as f64,
                soniclaw_dsp::resample::ResampleQuality::SincBest,
            )?;
            a.samples = Arc::from(data);
            a.sample_rate = rate;
        }
    }
    Ok(Preset {
        name: carrier.name,
        instrument: t.instrument.clone(),
        inserts: t.inserts.clone(),
        values: carrier.document.parameter_values.clone(),
        assets,
    })
}
pub fn install_preset(p: &mut Project, track: &str, preset: Preset) -> Result<(), ProjectError> {
    let mut next = p.clone();
    let mut d = preset
        .instrument
        .into_iter()
        .chain(preset.inserts)
        .next()
        .ok_or("preset is empty")?;
    let old = format!("{}.", d.id);
    d.id = workspace::identity(if d.kind == "stock.fx" { "fx" } else { "synth" })?;
    if d.kind == "stock.sampler" {
        let asset = d.state["asset"].as_str().ok_or("preset asset missing")?;
        let value = preset
            .assets
            .get(asset)
            .ok_or("preset PCM missing")?
            .clone();
        let id = workspace::identity("sample")?;
        next.assets.insert(id.clone(), value);
        d.state["asset"] = serde_json::Value::String(id);
    }
    let prefix = format!("{}.", d.id);
    if d.kind == "stock.fx" {
        crate::stock::add_effect(&mut next, track, d)?;
    } else {
        crate::stock::set_instrument(&mut next, track, d)?;
    }
    for (key, v) in preset.values {
        if let Some(suffix) = key.strip_prefix(&old) {
            next.document
                .parameter_values
                .insert(format!("{prefix}{suffix}"), v);
        }
    }
    next.validate()?;
    *p = next;
    Ok(())
}
/// Prepared bounded audition player; never changes a project and never reads a file in process().
pub struct Preview {
    asset: soniclaw_clips::AudioAsset,
    cursor: usize,
    pub done: Arc<AtomicBool>,
}
impl Preview {
    pub fn new(asset: soniclaw_clips::AudioAsset) -> Self {
        Self {
            asset,
            cursor: 0,
            done: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl AudioProcessor for Preview {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.channels != 2 || s.sample_rate != self.asset.sample_rate as f64 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        b.clear();
        if b.channels() != 2 {
            return;
        }
        for out in b.samples_mut().as_chunks_mut::<2>().0 {
            if self.cursor >= self.asset.frames().min(self.asset.sample_rate as u64 * 30) as usize {
                self.done.store(true, Ordering::Release);
                break;
            }
            for (c, v) in out.iter_mut().enumerate() {
                *v = self.asset.samples[self.cursor * self.asset.channels as usize
                    + if self.asset.channels == 1 { 0 } else { c }]
                    * 0.5;
            }
            self.cursor += 1;
        }
    }
    fn reset(&mut self) {
        self.cursor = 0;
        self.done.store(false, Ordering::Release);
    }
}
