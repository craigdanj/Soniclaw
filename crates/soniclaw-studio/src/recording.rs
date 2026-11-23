//! Prepared studio recording: callback-clock count-in, arming, pre-roll, punch and monitor-only click.
use crate::{
    project::{Project, ProjectError},
    *,
};
use soniclaw_record::multitrack::{self, Recorder, TakePlan};
use std::{
    path::Path,
    sync::atomic::{AtomicU8, AtomicUsize},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    RenderedTrack,
    DryInput,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<u16>,
    pub track: String,
    pub source: Source,
    pub monitor_input: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema: u32,
    pub arms: Vec<Arm>,
    pub master: bool,
    pub start_beat: f64,
    pub count_in_beats: u8,
    pub pre_roll_seconds: f64,
    pub punch_beats: Option<[f64; 2]>,
    pub metronome: bool,
    pub click_gain: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            arms: vec![],
            master: false,
            start_beat: 0.,
            count_in_beats: 0,
            pre_roll_seconds: 0.,
            punch_beats: None,
            metronome: false,
            click_gain: 0.1,
        }
    }
}
impl Settings {
    pub fn for_document(d: &Document) -> Self {
        Self {
            arms: d
                .tracks
                .iter()
                .map(|t| Arm {
                    channels: vec![],
                    track: t.id.clone(),
                    source: Source::RenderedTrack,
                    monitor_input: false,
                })
                .collect(),
            master: d.tracks.len() < 32,
            ..Default::default()
        }
    }
    pub fn validate(&self, d: &Document) -> Result<(), ProjectError> {
        d.validate()?;
        if self.schema != 1
            || self.arms.len() + usize::from(self.master) == 0
            || self.arms.len() + usize::from(self.master) > 32
            || self.count_in_beats > 32
            || !self.start_beat.is_finite()
            || !(0.0..=1_000_000.).contains(&self.start_beat)
            || !self.pre_roll_seconds.is_finite()
            || !(0.0..=10.).contains(&self.pre_roll_seconds)
            || !self.click_gain.is_finite()
            || !(0.0..=0.25).contains(&self.click_gain)
        {
            return Err("invalid recording controls/bounds".into());
        }
        let mut ids = BTreeSet::new();
        for a in &self.arms {
            if a.channels.len() > 2
                || a.channels.iter().any(|c| *c >= 64)
                || !ids.insert(&a.track)
                || !d.tracks.iter().any(|t| t.id == a.track)
                || self.master && a.track == "master"
            {
                return Err(
                    "unknown/duplicate recording track or master identity collision".into(),
                );
            }
        }
        if let Some([a, b]) = self.punch_beats {
            if !a.is_finite()
                || !b.is_finite()
                || a < 0.
                || a >= b
                || b > 1_000_000.
                || d.timeline.loop_range.is_some()
            {
                return Err(
                    "single-pass punch requires increasing beats and disabled arrangement looping"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn to_json(&self, d: &Document) -> Result<String, ProjectError> {
        self.validate(d)?;
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str, d: &Document) -> Result<Self, ProjectError> {
        if s.len() > 16384 {
            return Err("recording settings exceed 16 KiB".into());
        }
        let p: Self = serde_json::from_str(s)?;
        p.validate(d)?;
        Ok(p)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub calibration: Option<crate::timing::Calibration>,
    pub schema: u32,
    pub settings: Settings,
    pub project_start_frame: u64,
    pub count_in_frames: u64,
    pub pre_roll_frames: usize,
    pub acquisition_punch: [u64; 2],
    pub latency_tail_frames: usize,
    pub automatic_finish_frame: Option<u64>,
    pub sample_rate: u32,
}
#[derive(Default)]
pub struct Progress {
    phase: AtomicU8,
    count_in_remaining: AtomicU64,
    captured: AtomicU64,
    done: AtomicBool,
    metronome: AtomicBool,
    armed_stems: AtomicUsize,
    input_discontinuities: AtomicU64,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Status {
    pub phase: u8,
    pub count_in_remaining: u64,
    pub captured_frames: u64,
    pub punch_complete: bool,
    pub armed_stems: usize,
    pub input_discontinuities: u64,
}
impl Progress {
    pub fn status(&self) -> Status {
        Status {
            phase: self.phase.load(Ordering::Acquire),
            count_in_remaining: self.count_in_remaining.load(Ordering::Relaxed),
            captured_frames: self.captured.load(Ordering::Relaxed),
            punch_complete: self.done.load(Ordering::Acquire),
            armed_stems: self.armed_stems.load(Ordering::Relaxed),
            input_discontinuities: self.input_discontinuities.load(Ordering::Relaxed),
        }
    }
    pub fn set_metronome(&self, on: bool) {
        self.metronome.store(on, Ordering::Release);
    }
}
pub struct ArmedSession {
    pub controller: Controller,
    pub processor: RecordingProcessor,
    pub recorder: Recorder,
    pub progress: Arc<Progress>,
    pub schedule: Schedule,
}
/// Create a fresh recording engine and open a new take before publishing audio. Existing playback must be stopped.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    project: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    directory: &Path,
    take_id: &str,
    latency_budget: usize,
) -> Result<ArmedSession, ProjectError> {
    prepare_with_inputs(
        project,
        factory,
        settings,
        directory,
        take_id,
        latency_budget,
        None,
    )
}
/// Physical-channel variant. The device callback must fill the same bus before processing audio.
#[allow(clippy::too_many_arguments)]
pub fn prepare_with_inputs(
    project: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    directory: &Path,
    take_id: &str,
    latency_budget: usize,
    input: Option<Arc<ChannelBus>>,
) -> Result<ArmedSession, ProjectError> {
    prepare_internal(
        project,
        factory,
        settings,
        directory,
        take_id,
        Some(latency_budget),
        input,
    )
}
/// Prepare monitoring with exactly the compiled processor latency. Reprepare after topology changes.
pub fn prepare_low_latency_with_inputs(
    project: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    directory: &Path,
    take_id: &str,
    input: Option<Arc<ChannelBus>>,
) -> Result<ArmedSession, ProjectError> {
    prepare_internal(project, factory, settings, directory, take_id, None, input)
}
#[allow(clippy::too_many_arguments)]
fn prepare_internal(
    project: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    directory: &Path,
    take_id: &str,
    latency_budget: Option<usize>,
    input: Option<Arc<ChannelBus>>,
) -> Result<ArmedSession, ProjectError> {
    project.validate()?;
    settings.validate(&project.document)?;
    let mut document = project.document.clone();
    for t in &mut document.tracks {
        t.input_gain = if settings
            .arms
            .iter()
            .any(|a| a.track == t.id && a.monitor_input)
        {
            1.
        } else {
            0.
        };
    }
    let map = Arc::new(TempoMap::compile(&document.timeline)?);
    let anchor = map.sample_at(
        settings.punch_beats.map_or(settings.start_beat, |p| p[0]),
        document.sample_rate as f64,
    )?;
    let pre =
        ((settings.pre_roll_seconds * document.sample_rate as f64).round() as u64).min(anchor);
    let start = anchor - pre;
    let beat = map.beat_at_sample(start, document.sample_rate as f64)?;
    let bpm = map.position(beat)?.bpm;
    let count =
        (settings.count_in_beats as f64 * 60. / bpm * document.sample_rate as f64).round() as u64;
    let prepared = if let Some(bus) = input {
        let routes = settings
            .arms
            .iter()
            .filter(|a| a.monitor_input && !a.channels.is_empty())
            .map(|a| (a.track.clone(), a.channels.clone()))
            .collect();
        document.compile_inputs_at(
            &project.assets,
            &project.disk,
            factory,
            start,
            &crate::inputs::Setup { bus, routes },
        )?
    } else {
        if settings.arms.iter().any(|a| !a.channels.is_empty()) {
            return Err("physical channel routes require an input bus".into());
        }
        document.compile_streamed_at(&project.assets, &project.disk, factory, start)?
    };
    let budget = latency_budget.unwrap_or(prepared.latency_frames());
    let (mut controller, processor) = live_pair(prepared, budget, 1)?;
    let mut sources: Vec<_> = settings
        .arms
        .iter()
        .map(|a| {
            (
                a.track.clone(),
                match a.source {
                    Source::RenderedTrack => RecordingSource::Track(a.track.clone()),
                    Source::DryInput => {
                        if a.channels.is_empty() {
                            RecordingSource::Input
                        } else {
                            RecordingSource::InputChannels(a.channels.clone())
                        }
                    }
                },
            )
        })
        .collect();
    if settings.master {
        sources.push(("master".into(), RecordingSource::Master));
    }
    let specs = controller.recording_specs(&sources)?;
    let compensation = project
        .layout
        .calibration
        .as_ref()
        .map_or(0, |c| c.audio_offset_frames.max(0) as usize);
    let tail = specs
        .iter()
        .map(|s| s.latency_frames)
        .max()
        .unwrap_or(0)
        .saturating_add(compensation);
    let end = if let Some([_, b]) = settings.punch_beats {
        Some(
            count
                .checked_add(map.sample_at(b, document.sample_rate as f64)? - start)
                .and_then(|n| n.checked_add(tail as u64))
                .ok_or("recording range overflow")?,
        )
    } else {
        None
    };
    let schedule = Schedule {
        calibration: project.layout.calibration.clone(),
        schema: 1,
        settings: settings.clone(),
        project_start_frame: start,
        count_in_frames: count,
        pre_roll_frames: pre as usize,
        acquisition_punch: [count + pre, end.unwrap_or(1 << 48)],
        latency_tail_frames: tail,
        automatic_finish_frame: end,
        sample_rate: document.sample_rate,
    };
    let (mut recorder, tap) = multitrack::pair(multitrack::Config {
        sample_rate: document.sample_rate,
        tracks: specs,
        pre_roll_frames: pre as usize,
        queue_packets: 256,
        max_block_frames: document.max_block_frames,
    })?;
    controller.seek(start)?;
    controller.attach_multitrack_before_start(
        tap,
        &sources.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(),
    )?;
    recorder.start(
        directory,
        TakePlan {
            id: take_id.into(),
            label: "Studio recording".into(),
            punch: Some(schedule.acquisition_punch),
        },
    )?;
    let progress = Arc::new(Progress::default());
    progress
        .metronome
        .store(settings.metronome, Ordering::Release);
    progress.armed_stems.store(sources.len(), Ordering::Relaxed);
    progress.count_in_remaining.store(count, Ordering::Relaxed);
    let mut click_at = [u64::MAX; 32];
    for (i, at) in click_at
        .iter_mut()
        .enumerate()
        .take(settings.count_in_beats as usize)
    {
        *at = (i as f64 * 60. / bpm * document.sample_rate as f64).round() as u64;
    }
    let audio = RecordingProcessor {
        inner: processor,
        progress: progress.clone(),
        schedule: schedule.clone(),
        map,
        elapsed: 0,
        click_at,
        click_left: 0,
        click_age: 0,
        accent: false,
        last_click_source: None,
        events: [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; 1024],
    };
    Ok(ArmedSession {
        controller,
        processor: audio,
        recorder,
        progress,
        schedule,
    })
}
pub struct RecordingProcessor {
    inner: SessionProcessor,
    progress: Arc<Progress>,
    schedule: Schedule,
    map: Arc<TempoMap>,
    elapsed: u64,
    click_at: [u64; 32],
    click_left: usize,
    click_age: usize,
    accent: bool,
    last_click_source: Option<u64>,
    events: [Event; 1024],
}
impl RecordingProcessor {
    fn click(&mut self, output: &mut [f32], counting: bool, offset: u64) {
        let rate = self.schedule.sample_rate as f64;
        for (f, frame) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let source = self.inner.capture.clock_at(f);
            let trigger = if counting {
                let sample = offset + f as u64;
                self.click_at
                    .iter()
                    .position(|&s| s == sample)
                    .map(|i| i == 0)
            } else if self.progress.metronome.load(Ordering::Relaxed) {
                source.and_then(|(sample, playing, looped)| {
                    if !playing {
                        return None;
                    }
                    let beat = self.map.beat_at_sample(sample, rate).ok()?.round();
                    let at = self.map.sample_at(beat, rate).ok()?;
                    if at == sample && (self.last_click_source != Some(sample) || looped) {
                        self.last_click_source = Some(sample);
                        Some(
                            self.map
                                .position(beat)
                                .is_ok_and(|p| p.beat_in_bar.abs() < 1e-8),
                        )
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            if let Some(accent) = trigger {
                self.accent = accent;
                self.click_left = (rate * 0.02) as usize;
                self.click_age = 0;
            }
            if self.click_left > 0 {
                let age = self.click_age as f64 / rate;
                let ramp = (age / 0.001).min(1.);
                let envelope = (-age * 200.).exp() * ramp;
                let hz = if self.accent { 1600. } else { 1000. };
                let x = (std::f64::consts::TAU * hz * age).sin() as f32
                    * envelope as f32
                    * self.schedule.settings.click_gain;
                frame[0] += x;
                frame[1] += x;
                if let (Some(output), Some(gains)) = (&self.inner.output_bus, &self.inner.click_bus)
                {
                    for (i, meter) in self.inner.cue_meters.iter().enumerate() {
                        let mut click = [x, x];
                        self.inner.cue_click_delays[i].process(&mut click);
                        let _ = output.add_sample(f, 2 + i * 2, click[0] * gains.sample(f, i * 2));
                        let _ =
                            output.add_sample(f, 3 + i * 2, click[1] * gains.sample(f, i * 2 + 1));
                        meter.publish([
                            output.sample(f, 2 + i * 2).abs(),
                            output.sample(f, 3 + i * 2).abs(),
                        ]);
                    }
                }
                self.click_age += 1;
                self.click_left -= 1;
            }
        }
    }
}
impl AudioProcessor for RecordingProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.inner.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != 2
            || b.frames() > self.inner.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        if let Some(bus) = &self.inner.capture.input {
            self.progress
                .input_discontinuities
                .store(bus.discontinuities(), Ordering::Relaxed);
            if !bus.ready() && self.elapsed == 0 {
                b.clear();
                return;
            }
        }
        let frames = b.frames();
        let mut offset = 0;
        while offset < frames {
            let counting = self.elapsed < self.schedule.count_in_frames;
            let mut count = frames - offset;
            if counting {
                count = count.min((self.schedule.count_in_frames - self.elapsed) as usize);
            }
            let mut events = 0;
            for e in ctx.events {
                if e.frame >= offset && e.frame < offset + count {
                    self.events[events] = Event {
                        frame: e.frame - offset,
                        ..*e
                    };
                    events += 1;
                }
            }
            if let Some(bus) = &self.inner.capture.input {
                let _ = bus.set_offset(offset);
            }
            if let Some(bus) = &self.inner.output_bus {
                let _ = bus.set_offset(offset);
            }
            if let Some(bus) = &self.inner.click_bus {
                let _ = bus.set_offset(offset);
            }
            self.inner.playing = !counting;
            let slice = &mut b.samples_mut()[offset * 2..(offset + count) * 2];
            self.inner.process(
                &mut AudioBuffer::new(slice, 2).expect("stereo"),
                &ProcessContext {
                    events: &self.events[..events],
                    transport: ctx.transport,
                },
            );
            self.click(slice, counting, self.elapsed);
            self.elapsed = self.elapsed.saturating_add(count as u64);
            offset += count;
        }
        if let Some(bus) = &self.inner.capture.input {
            let _ = bus.set_offset(0);
        }
        if let Some(bus) = &self.inner.output_bus {
            let _ = bus.set_offset(0);
        }
        if let Some(bus) = &self.inner.click_bus {
            let _ = bus.set_offset(0);
        }
        self.progress.count_in_remaining.store(
            self.schedule.count_in_frames.saturating_sub(self.elapsed),
            Ordering::Relaxed,
        );
        let end = self.schedule.automatic_finish_frame.unwrap_or(u64::MAX);
        let captured = self
            .elapsed
            .min(end)
            .saturating_sub(self.schedule.count_in_frames);
        self.progress.captured.store(captured, Ordering::Relaxed);
        let done = self.elapsed >= end;
        self.progress.phase.store(
            if done {
                2
            } else if self.elapsed < self.schedule.count_in_frames {
                0
            } else {
                1
            },
            Ordering::Release,
        );
        self.progress.done.store(done, Ordering::Release);
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.click_left = 0;
        self.last_click_source = None;
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
}
/// Place gap-free dry-input stems on their armed tracks. Raw/rendered stems remain available separately.
/// Loop recordings use take-lane recipes instead of flattening repeated passes onto the arrangement.
pub fn place_input_take(
    project: &mut Project,
    path: &Path,
    schedule: &Schedule,
) -> Result<usize, ProjectError> {
    use sha2::{Digest, Sha256};
    let take = multitrack::Take::read(path)?;
    if take.sample_rate != project.document.sample_rate
        || schedule.sample_rate != take.sample_rate
        || take.missing_frames != 0
        || take.queue_drops != 0
        || take.input_discontinuities != 0
        || project.document.timeline.loop_range.is_some()
    {
        return Err("take is not a gap-free linear project-rate recording".into());
    }
    schedule.settings.validate(&project.document)?;
    if let Some(c) = &schedule.calibration {
        c.validate(take.sample_rate)?;
    }
    let mut next = project.clone();
    let mut count = 0;
    for arm in schedule
        .settings
        .arms
        .iter()
        .filter(|a| a.source == Source::DryInput)
    {
        let stem = take
            .tracks
            .iter()
            .find(|t| t.id == arm.track)
            .ok_or("take stem missing")?;
        if stem.channels > 2 {
            return Err("input stem exceeds stereo".into());
        }
        let key = format!(
            "recording.{}",
            &format!("{:x}", Sha256::digest(format!("{}:{}", take.id, stem.id)))[..32]
        );
        if next.disk.contains_key(&key) {
            return Err("recording already placed".into());
        }
        let start = schedule
            .project_start_frame
            .checked_add(take.file_start)
            .ok_or("placement overflow")?;
        let raw_start = start.saturating_sub(schedule.count_in_frames);
        let compensation = schedule
            .calibration
            .as_ref()
            .map_or(0, |c| c.audio_offset_frames);
        let (timeline_start, trim) = crate::timing::placement(raw_start, compensation)?;
        let skipped = schedule
            .count_in_frames
            .saturating_sub(start)
            .saturating_add(trim);
        let offset = (stem.latency_frames as u64)
            .checked_add(skipped)
            .ok_or("placement overflow")?;
        let length = take
            .frames
            .checked_sub(offset)
            .filter(|n| *n > 0)
            .ok_or("take shorter than placement compensation")?;
        let clip = DiskClip {
            id: format!("{key}.clip"),
            asset: key.clone(),
            start_frame: timeline_start,
            length_frames: length,
            source_offset: offset,
            source_length: length,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
            edits: None,
        };
        let track = next
            .document
            .tracks
            .iter_mut()
            .find(|t| t.id == arm.track)
            .ok_or("armed track removed")?;
        track.streamed.push(clip);
        next.disk.insert(key, path.join(&stem.file));
        count += 1;
    }
    next.validate()?;
    *project = next;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(root: &Path, block: usize, source: Source, monitor: bool) -> (Vec<f32>, Schedule, u64) {
        let (mut d, a) = crate::tests::fixture();
        d.max_block_frames = 256;
        d.tracks[0].audio.clear();
        d.timeline.tempo[0].bpm = 400.;
        let p = Project::new("Recording".into(), d, a, Default::default()).unwrap();
        let settings = Settings {
            arms: vec![Arm {
                channels: vec![],
                track: "audio".into(),
                source,
                monitor_input: monitor,
            }],
            master: true,
            count_in_beats: 1,
            pre_roll_seconds: 0.01,
            punch_beats: Some([0.5, 1.5]),
            metronome: true,
            ..Default::default()
        };
        let mut armed = prepare(&p, &mut Builtins, settings, root, "take", 64).unwrap();
        let mut data = vec![0.2; block * 2];
        let mut clicks = 0;
        let allocation = allocation_counter::measure(|| {
            while !armed.progress.status().punch_complete {
                data.fill(0.2);
                armed.processor.process(
                    &mut AudioBuffer::new(&mut data, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Default::default(),
                    },
                );
                clicks += usize::from(data.iter().any(|v| v.abs() > 0.001));
            }
        });
        assert_eq!(allocation.count_total, 0);
        assert!(clicks > 0);
        let take = armed.recorder.stop().unwrap();
        assert!(take.complete);
        assert_eq!(take.missing_frames, 0);
        assert_eq!(take.file_start, armed.schedule.count_in_frames);
        assert_eq!(take.frames, 7744);
        let pcm = hound::WavReader::open(root.join("audio.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let master = hound::WavReader::open(root.join("master.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        if source == Source::DryInput && !monitor {
            assert!(pcm.iter().all(|&x| x == 0.2));
            assert!(master.iter().all(|&x| x == 0.));
        }
        armed.controller.detach_multitrack().unwrap();
        data.fill(0.);
        armed.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        armed.controller.collect_retired();
        (pcm, armed.schedule, take.frames)
    }
    #[test]
    fn count_in_preroll_punch_and_click_exclusion_are_partition_invariant() {
        let root =
            std::env::temp_dir().join(format!("soniclaw-recording62-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let (a, plan, frames) = run(&root.join("a"), 127, Source::DryInput, false);
        let (b, _, _) = run(&root.join("b"), 64, Source::DryInput, false);
        assert_eq!(a, b);
        assert_eq!(plan.count_in_frames, 7200);
        assert_eq!(plan.pre_roll_frames, 480);
        assert_eq!(frames, 7744);
        let (c, _, _) = run(&root.join("monitor"), 127, Source::RenderedTrack, true);
        assert!(c[256..].iter().all(|&x| (x - 0.1).abs() < 1e-6));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn controls_reject_invalid_arming_loop_punch_and_roundtrip() {
        let (d, _) = crate::tests::fixture();
        let settings = Settings::for_document(&d);
        let text = settings.to_json(&d).unwrap();
        assert_eq!(Settings::from_json(&text, &d).unwrap().arms.len(), 1);
        let mut bad = settings.clone();
        bad.arms.push(bad.arms[0].clone());
        assert!(bad.validate(&d).is_err());
        bad = settings;
        bad.punch_beats = Some([0., 1.]);
        let mut looping = d;
        looping.timeline.loop_range = Some([0., 4.]);
        assert!(bad.validate(&looping).is_err());
    }
}
#[cfg(test)]
mod seek_recording_tests {
    use super::*;
    #[test]
    fn streamed_recording_is_primed_at_selected_start_before_first_callback() {
        let root =
            std::env::temp_dir().join(format!("soniclaw-record-prime70-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.wav");
        let mut w = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..96000 {
            w.write_sample(0.3f32).unwrap();
        }
        w.finalize().unwrap();
        let (mut d, a) = crate::tests::fixture();
        d.tracks[0].audio.clear();
        d.tracks[0].streamed.push(DiskClip {
            edits: None,
            id: "disk-region".into(),
            asset: "disk".into(),
            start_frame: 0,
            length_frames: 96000,
            source_offset: 0,
            source_length: 96000,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
        });
        let p = Project::new(
            "Seek recording".into(),
            d,
            a,
            [("disk".into(), source)].into(),
        )
        .unwrap();
        let settings = Settings {
            arms: vec![Arm {
                channels: vec![],
                track: "audio".into(),
                source: Source::RenderedTrack,
                monitor_input: false,
            }],
            start_beat: 2.,
            punch_beats: Some([2., 2.1]),
            ..Default::default()
        };
        let mut armed =
            prepare(&p, &mut Builtins, settings, &root.join("take"), "take", 64).unwrap();
        let mut block = vec![0.; 512];
        armed.processor.process(
            &mut AudioBuffer::new(&mut block, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        armed.recorder.stop().unwrap();
        let pcm = hound::WavReader::open(root.join("take/audio.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!pcm.is_empty());
        assert!(
            pcm.iter().all(|&s| (s - 0.15).abs() < 1e-6),
            "first captured callback must contain the selected disk source, not startup starvation"
        );
        drop(armed);
        std::fs::remove_dir_all(root).unwrap();
    }
}
