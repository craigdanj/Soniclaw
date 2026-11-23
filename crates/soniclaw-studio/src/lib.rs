//! Shared timeline/clip/instrument/insert/mixer runtime. Prepare and retire on the control thread.
pub mod arrangement;
pub mod automation_write;
pub mod cues;
pub mod delivery;
pub mod elastic;
pub mod export;
pub mod export_convert;
pub mod freeze;
pub mod history;
pub mod inputs;
pub mod jobs;
pub mod media;
pub mod media_tools;
pub mod midi_edit;
mod midi_pass;
pub mod midi_recording;
pub mod mix_graph;
pub mod native_buses;
pub mod native_edits;
pub mod native_state;
pub mod pending;
pub mod precision;
pub mod project;
pub mod recording;
pub mod recovery;
pub mod routing;
pub mod sessions;
pub mod stock;
pub mod takes;
pub mod tempo_cache;
pub mod timing;
pub mod wave_edit;
pub mod workspace;
use serde::{Deserialize, Serialize};
use soniclaw_automation::{AutomatedProcessor, Document as Automation, Gesture};
use soniclaw_clips::{Assets, AudioClip, MidiClip, Player};
use soniclaw_core::*;
mod capture;
pub use capture::RecordingSource;
use capture::{CaptureStore, RecordingBinding};
use soniclaw_graph::live::{graph_pair, GraphController, LiveGraph};
use soniclaw_mixer::{Controls, Mixer, Model as MixerModel};
use soniclaw_stream::clips::{DiskAssets, DiskClip, StreamTelemetry, StreamedClips};
use soniclaw_transport::{Clock, TempoMap, Timeline};
use std::sync::atomic::AtomicBool;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"._-".contains(&v))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub id: String,
    pub kind: String,
    pub state: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub streamed: Vec<DiskClip>,
    pub id: String,
    pub audio: Vec<AudioClip>,
    pub midi: Vec<MidiClip>,
    pub instrument: Option<Descriptor>,
    pub inserts: Vec<Descriptor>,
    pub input_gain: f32,
    pub midi_input: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auxiliary {
    pub inserts: Vec<Descriptor>,
    pub return_gain: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<cues::Cue>,
    #[serde(default, skip_serializing_if = "mix_graph::Model::is_empty")]
    pub mix_routing: mix_graph::Model,
    #[serde(default)]
    pub parameter_values: BTreeMap<String, f32>,
    pub schema: u32,
    pub sample_rate: u32,
    pub max_block_frames: usize,
    pub timeline: Timeline,
    pub tracks: Vec<Track>,
    pub mixer: MixerModel,
    pub auxiliaries: Vec<Auxiliary>,
    pub automation: Automation,
}
impl Document {
    pub fn spec(&self) -> ProcessSpec {
        ProcessSpec {
            sample_rate: self.sample_rate as f64,
            max_block_frames: self.max_block_frames,
            channels: 2,
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !(8000..=192000).contains(&self.sample_rate)
            || !(1..=1024).contains(&self.max_block_frames)
            || self.tracks.is_empty()
            || self.tracks.len() > 32
            || self.tracks.len() != self.mixer.strips.len()
            || self.auxiliaries.len() != self.mixer.taps.len()
            || self
                .tracks
                .iter()
                .map(|t| t.audio.len() + t.midi.len())
                .sum::<usize>()
                > 1024
        {
            return Err(Error::InvalidSpec);
        }
        if self.parameter_values.len() > 4096
            || self.parameter_values.iter().any(|(id, value)| {
                id.is_empty()
                    || id.len() > 256
                    || !value.is_finite()
                    || !(0.0..=1.0).contains(value)
            })
        {
            return Err(Error::InvalidParameter);
        }
        if self.tracks.iter().map(|t| t.streamed.len()).sum::<usize>() > 16 {
            return Err(Error::InvalidSpec);
        }
        for t in &self.tracks {
            let mut ids = BTreeSet::new();
            for clip in &t.streamed {
                clip.validate()?;
                if !ids.insert(&clip.id) {
                    return Err(Error::InvalidParameter);
                }
            }
        }
        self.mixer.validate()?;
        self.automation.validate()?;
        TempoMap::compile(&self.timeline)?;
        let mut ids = BTreeSet::new();
        let mut processors = BTreeSet::new();
        for (t, m) in self.tracks.iter().zip(&self.mixer.strips) {
            if !valid_id(&t.id)
                || !ids.insert(&t.id)
                || m.id != t.id
                || t.inserts.len() > 16
                || !t.input_gain.is_finite()
                || t.input_gain.abs() > 4.
            {
                return Err(Error::InvalidRouting);
            }
            self.clip_document(t).validate()?;
            for p in t.instrument.iter().chain(&t.inserts) {
                validate_descriptor(p, &mut processors)?;
            }
        }
        for a in &self.auxiliaries {
            if a.inserts.len() > 16 || !a.return_gain.is_finite() || a.return_gain.abs() > 4. {
                return Err(Error::InvalidRouting);
            }
            for p in &a.inserts {
                validate_descriptor(p, &mut processors)?;
            }
        }
        self.mix_routing.validate(self)?;
        cues::validate(self)?;
        for p in self.mix_routing.descriptors() {
            validate_descriptor(p, &mut processors)?;
        }
        Ok(())
    }
    fn clip_document(&self, t: &Track) -> soniclaw_clips::Document {
        soniclaw_clips::Document {
            schema: 1,
            sample_rate: self.sample_rate,
            channels: 2,
            timeline: self.timeline.clone(),
            audio: t.audio.clone(),
            midi: t.midi.clone(),
        }
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        let s = serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)?;
        if s.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        Ok(s)
    }
    pub fn from_json(s: &str) -> Result<Self, Error> {
        if s.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        let d: Self = serde_json::from_str(s).map_err(|_| Error::InvalidParameter)?;
        d.validate()?;
        Ok(d)
    }
    pub fn compile(
        &self,
        assets: &Assets,
        factory: &mut impl Factory,
    ) -> Result<PreparedSession, Error> {
        self.compile_streamed(assets, &DiskAssets::new(), factory)
    }
    pub fn compile_streamed(
        &self,
        assets: &Assets,
        disk_assets: &DiskAssets,
        factory: &mut impl Factory,
    ) -> Result<PreparedSession, Error> {
        self.compile_mode(assets, disk_assets, factory, false, 0, None)
    }
    /// Prepare disk pages for a stopped transport's chosen starting sample.
    /// The caller still supplies that position via its transport or Controller::seek before publication.
    pub fn compile_streamed_at(
        &self,
        assets: &Assets,
        disk_assets: &DiskAssets,
        factory: &mut impl Factory,
        start: u64,
    ) -> Result<PreparedSession, Error> {
        if start > 1 << 48 {
            return Err(Error::InvalidParameter);
        }
        self.compile_mode(assets, disk_assets, factory, false, start, None)
    }
    /// Typed offline renderer. Never implements AudioProcessor or exposes its prepared instance.
    pub fn compile_offline(
        &self,
        assets: &Assets,
        disk_assets: &DiskAssets,
        factory: &mut impl Factory,
    ) -> Result<OfflineSession, Error> {
        Ok(OfflineSession {
            inner: self.compile_mode(assets, disk_assets, factory, true, 0, None)?,
        })
    }
    fn compile_mode(
        &self,
        assets: &Assets,
        disk_assets: &DiskAssets,
        factory: &mut impl Factory,
        offline: bool,
        prime_sample: u64,
        inputs: Option<&inputs::Setup>,
    ) -> Result<PreparedSession, Error> {
        self.validate()?;
        if let Some(i) = inputs {
            i.validate(self)?;
        }
        let spec = self.spec();
        let mut parameters = Vec::new();
        factory.prepare_document(self)?;
        let mut indexed_tracks = Vec::new();
        let track_order = self.mix_routing.sidechain_order(self)?;
        let mut detector_buses = Vec::new();
        for t in &self.tracks {
            detector_buses.push(
                if self.mix_routing.sidechains.iter().any(|s| s.source == t.id) {
                    Some(Arc::new(ChannelBus::new(2, spec.max_block_frames)?))
                } else {
                    None
                },
            );
        }
        let mut detectors = BTreeMap::new();
        for sc in &self.mix_routing.sidechains {
            let i = self
                .tracks
                .iter()
                .position(|t| t.id == sc.source)
                .ok_or(Error::InvalidRouting)?;
            detectors.insert(
                sc.target.clone(),
                detector_buses[i].clone().ok_or(Error::InvalidRouting)?,
            );
        }
        let mut track_latency = 0;
        let mut delay_samples = 0usize;
        for &track_index in &track_order {
            let t = &self.tracks[track_index];
            let player = self
                .clip_document(t)
                .compile(assets, spec.max_block_frames, None)?;
            let streamed = if t.streamed.is_empty() {
                None
            } else {
                let mut streams = StreamedClips::new(&t.streamed, disk_assets, spec, 1024 * 1024)
                    .map_err(|_| Error::InvalidBuffer)?;
                if !streams.prime_at(prime_sample, std::time::Duration::from_millis(500)) {
                    return Err(Error::NotPrepared);
                }
                Some(streams)
            };
            let instrument = t
                .instrument
                .as_ref()
                .map(|d| build(d, factory, spec, &mut parameters))
                .transpose()?;
            let own = instrument.as_ref().map_or(0, |p| p.latency_frames());
            let mut latency = own;
            let mut prefix_latency = own;
            let inserts = t
                .inserts
                .iter()
                .map(|d| {
                    if matches!(d.kind.as_str(), "vst3" | "clap")
                        && d.state.get("buses").is_some()
                        && prefix_latency != 0
                    {
                        return Err(Error::InvalidRouting);
                    }
                    let p = if let Some(bus) = detectors.get(&d.id) {
                        let mut p = mix_graph::sidechain(d, bus.clone())?;
                        p.processor.prepare(spec)?;
                        parameters.extend(p.parameters);
                        Ok(p.processor)
                    } else {
                        build(d, factory, spec, &mut parameters)
                    }?;
                    prefix_latency = prefix_latency
                        .checked_add(p.latency_frames())
                        .ok_or(Error::InvalidSpec)?;
                    Ok(p)
                })
                .collect::<Result<Vec<_>, _>>()?;
            for p in &inserts {
                latency = latency
                    .checked_add(p.latency_frames())
                    .ok_or(Error::InvalidSpec)?;
            }
            track_latency = track_latency.max(latency);
            let monitor = self
                .cues
                .iter()
                .flat_map(|c| &c.sends)
                .any(|s| matches!(&s.source,cues::Source::TrackPre(id) if id==&t.id));
            let native_tap = factory.track_tap(&t.id);
            if let Some(tap) = &native_tap {
                tap.latency.store(latency, Ordering::Release);
            }
            indexed_tracks.push((
                track_index,
                TrackRuntime {
                    native_tap,
                    monitor_pcm: if monitor {
                        vec![0.; spec.max_block_frames * 2]
                    } else {
                        Vec::new()
                    },
                    sidechain: detector_buses[track_index].clone(),
                    external_input: inputs
                        .and_then(|i| i.routes.get(&t.id).map(|r| (i.bus.clone(), r.clone()))),
                    offline,
                    streamed,
                    player,
                    instrument,
                    inserts,
                    input_gain: t.input_gain,
                    midi_input: t.midi_input,
                    source_delay: Delay::new(own, &mut delay_samples)?,
                    align: Delay::default(),
                    latency,
                    pcm: vec![0.; spec.max_block_frames * 2],
                    instrument_pcm: vec![0.; spec.max_block_frames * 2],
                    events: Vec::with_capacity(1024),
                    previous: None,
                    panic: true,
                },
            ));
        }
        indexed_tracks.sort_by_key(|(index, _)| *index);
        let mut tracks = indexed_tracks
            .into_iter()
            .map(|(_, track)| track)
            .collect::<Vec<_>>();
        for sc in &self.mix_routing.sidechains {
            let source = self
                .tracks
                .iter()
                .position(|t| t.id == sc.source)
                .ok_or(Error::InvalidRouting)?;
            let target = self
                .tracks
                .iter()
                .position(|t| t.inserts.iter().any(|p| p.id == sc.target))
                .ok_or(Error::InvalidRouting)?;
            if tracks[source].latency != 0 || tracks[target].latency != 0 {
                return Err(Error::InvalidRouting);
            }
        }
        for t in &mut tracks {
            t.align = Delay::new(track_latency - t.latency, &mut delay_samples)?;
        }
        let mut auxiliaries = Vec::new();
        let mut aux_latency = 0;
        for (i, a) in self.auxiliaries.iter().enumerate() {
            let inserts = a
                .inserts
                .iter()
                .map(|d| build(d, factory, spec, &mut parameters))
                .collect::<Result<Vec<_>, _>>()?;
            let latency = inserts.iter().try_fold(0usize, |sum, p| {
                sum.checked_add(p.latency_frames())
                    .ok_or(Error::InvalidSpec)
            })?;
            aux_latency = aux_latency.max(latency);
            let gain = Arc::new(Parameter::new(ParameterInfo {
                id: &format!("aux.{i}.return_gain"),
                name: "Aux return",
                min: -4.,
                max: 4.,
                default: a.return_gain,
                unit: "",
            })?);
            parameters.push(gain.clone());
            auxiliaries.push(AuxRuntime {
                inserts,
                gain,
                latency,
                align: Delay::default(),
            });
        }
        let main_delay = Delay::new(aux_latency, &mut delay_samples)?;
        for a in &mut auxiliaries {
            a.align = Delay::new(aux_latency - a.latency, &mut delay_samples)?;
        }
        let mut mixer = self
            .mixer
            .compile(spec.sample_rate, spec.max_block_frames)?;
        let mut bindings = Vec::new();
        for (t, c) in self.tracks.iter().zip(mixer.controls()) {
            let mut controls = Vec::new();
            for (key, min, max, default) in [
                ("gain", 0., 4., c.gain()),
                ("pan", -1., 1., c.pan()),
                ("mute", 0., 1., if c.muted() { 1. } else { 0. }),
                ("solo", 0., 1., if c.soloed() { 1. } else { 0. }),
            ] {
                let id = format!("mix.{}.{}", t.id, key);
                controls.push(Arc::new(Parameter::new(ParameterInfo {
                    id: &id,
                    name: key,
                    min,
                    max,
                    default,
                    unit: "",
                })?));
            }
            for (i, v) in self.mixer.strips[bindings.len()].sends.iter().enumerate() {
                let id = format!("mix.{}.send{}", t.id, i);
                controls.push(Arc::new(Parameter::new(ParameterInfo {
                    id: &id,
                    name: "Send",
                    min: 0.,
                    max: 4.,
                    default: *v,
                    unit: "",
                })?));
            }
            parameters.extend(controls.iter().cloned());
            bindings.push((c.clone(), controls));
        }
        let mut mix_routing = if self.mix_routing.is_empty() {
            None
        } else {
            Some(mix_graph::Runtime::new(
                self,
                factory,
                &mut parameters,
                track_latency,
                aux_latency,
                &mut delay_samples,
            )?)
        };
        let mut mix_meters = mix_routing
            .as_ref()
            .map_or_else(Vec::new, |r| r.meters.clone());
        let mut cues = if self.cues.is_empty() {
            None
        } else {
            Some(cues::Runtime::new(
                self,
                &tracks,
                mix_routing.as_ref(),
                track_latency,
                aux_latency,
                inputs.map(|i| i.bus.clone()),
                &mut parameters,
                &mut delay_samples,
                &mut mix_meters,
            )?)
        };
        let mut ids = BTreeSet::new();
        if parameters.len() > 4096 || parameters.iter().any(|p| !ids.insert(p.info().id)) {
            return Err(Error::InvalidParameter);
        }
        for (id, value) in &self.parameter_values {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(Error::InvalidParameter);
            }
            let p = parameters
                .iter()
                .find(|p| p.info().id == id)
                .ok_or(Error::InvalidParameter)?;
            p.set_normalized(*value)?;
        }
        if let Some(r) = &mut mix_routing {
            r.initialize_controls();
            mixer.initialize_audible(&r.audible_tracks()[..self.tracks.len()])?;
        }
        if let Some(c) = &mut cues {
            c.initialize_controls();
        }
        let latency = mix_routing
            .as_ref()
            .map_or(track_latency + aux_latency, |r| r.latency);
        if latency > 262144 {
            return Err(Error::InvalidSpec);
        }
        let streaming_stats = tracks
            .iter()
            .filter_map(|t| t.streamed.as_ref())
            .flat_map(|s| s.telemetry())
            .collect();
        let capture = Arc::new(CaptureStore::new(
            &tracks,
            self.tracks.iter().map(|t| t.id.clone()).collect(),
            spec.max_block_frames,
            track_latency,
            inputs.map(|i| i.bus.clone()),
        ));
        let click_bus = cues.as_ref().map(|c| c.click_bus.clone());
        let output_bus = cues.as_ref().map(|c| c.output.clone());
        let cue_latencies = cues.as_ref().map_or_else(Vec::new, |c| c.latencies.clone());
        let cue_click_delays = self
            .cues
            .iter()
            .zip(&cue_latencies)
            .map(|(c, (_, latency))| {
                Delay::new(
                    if c.align_sources && c.metronome {
                        *latency
                    } else {
                        0
                    },
                    &mut delay_samples,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let routed_taps = delivery::prepare(
            self,
            spec.max_block_frames,
            track_latency,
            aux_latency,
            mix_routing.as_ref(),
        )?;
        let renderer = Renderer {
            routed_taps: routed_taps.clone(),
            cues,
            mix_routing,
            track_order,
            post: vec![vec![0.; spec.max_block_frames * 2]; tracks.len()],
            capture: capture.clone(),
            tracks,
            mixer,
            bindings,
            auxiliaries,
            main_delay,
            sends: vec![vec![0.; spec.max_block_frames * 2]; self.auxiliaries.len()],
            input: vec![0.; spec.max_block_frames * 2],
            spec,
            latency,
            previous: None,
            failures: Arc::new(AtomicU64::new(0)),
        };
        let failures = renderer.failures.clone();
        let map = Arc::new(TempoMap::compile(&self.timeline)?);
        let (inner, gestures) =
            AutomatedProcessor::new(Box::new(renderer), self.automation.clone(), &parameters)?;
        let mut inner = inner.with_tempo_map(map.clone());
        inner.prepare(spec)?;
        let clock = Clock::from_map(&self.timeline, spec.sample_rate, map)?;
        Ok(PreparedSession {
            routed_taps,
            cue_click_delays,
            click_bus,
            output_bus,
            cue_latencies,
            mix_meters,
            offline,
            inner: Box::new(inner),
            clock,
            spec,
            latency,
            parameters,
            gestures,
            source_end: None,
            scratch: Vec::with_capacity(1024),
            state: Arc::new(State::default()),
            revision: 1,
            failures,
            document: self.clone(),
            capture,
            streaming_stats,
        })
    }
}
fn validate_descriptor<'a>(d: &'a Descriptor, ids: &mut BTreeSet<&'a str>) -> Result<(), Error> {
    if !valid_id(&d.id)
        || !valid_id(&d.kind)
        || !ids.insert(&d.id)
        || d.state.to_string().len() > 65536
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
pub struct Built {
    pub processor: Box<dyn AudioProcessor>,
    pub parameters: Vec<Arc<Parameter>>,
}
pub trait Factory {
    fn prepare_document(&mut self, _document: &Document) -> Result<(), Error> {
        Ok(())
    }
    fn track_tap(&self, _id: &str) -> Option<Arc<native_buses::Frames>> {
        None
    }
    fn create(&mut self, descriptor: &Descriptor) -> Result<Built, Error>;
}
impl<F> Factory for F
where
    F: FnMut(&Descriptor) -> Result<Built, Error>,
{
    fn create(&mut self, d: &Descriptor) -> Result<Built, Error> {
        self(d)
    }
}
fn build(
    d: &Descriptor,
    f: &mut impl Factory,
    s: ProcessSpec,
    params: &mut Vec<Arc<Parameter>>,
) -> Result<Box<dyn AudioProcessor>, Error> {
    let mut p = f.create(d)?;
    p.processor.prepare(s)?;
    if p.processor.latency_frames() > 262144 {
        return Err(Error::InvalidSpec);
    }
    params.extend(p.parameters);
    Ok(p.processor)
}
#[derive(Default)]
pub struct Builtins;
impl Factory for Builtins {
    fn create(&mut self, d: &Descriptor) -> Result<Built, Error> {
        if d.kind.starts_with("stock.") {
            return stock::build(d, None);
        }
        use soniclaw_dsp::*;
        let object = d.state.as_object().ok_or(Error::InvalidParameter)?;
        let number = |key: &str, default: f32| -> Result<f32, Error> {
            object.get(key).map_or(Ok(default), |v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .map(|v| v as f32)
                    .ok_or(Error::InvalidParameter)
            })
        };
        let allowed: &[&str] = match d.kind.as_str() {
            "sine" | "saw" => &[],
            "gain" => &["value"],
            "lowpass" => &["hz"],
            "delay" => &["seconds", "feedback", "mix"],
            _ => return Err(Error::InvalidParameter),
        };
        if object.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err(Error::InvalidParameter);
        }
        let mut parameters = Vec::new();
        let processor: Box<dyn AudioProcessor> = match d.kind.as_str() {
            "sine" | "saw" => Box::new(ExpressiveSynth::<32>::new(
                if d.kind == "sine" {
                    Waveform::Sine
                } else {
                    Waveform::Saw
                },
                AdsrSettings::default(),
            )?),
            "gain" | "lowpass" => {
                let (kind, min, max, default, unit) = if d.kind == "gain" {
                    ("gain", 0., 4., number("value", 1.)?, "")
                } else {
                    ("cutoff", 20., 20000., number("hz", 4000.)?, "Hz")
                };
                let id = format!("{}.{}", d.id, kind);
                let p = Arc::new(Parameter::new(ParameterInfo {
                    id: &id,
                    name: kind,
                    min,
                    max,
                    default,
                    unit,
                })?);
                parameters.push(p.clone());
                if d.kind == "gain" {
                    Box::new(Gain::new(p))
                } else {
                    Box::new(LowPass::new(p))
                }
            }
            "delay" => Box::new(Delay::new(
                number("seconds", 0.25)?,
                number("feedback", 0.2)?,
                number("mix", 0.3)?,
            )?),
            _ => unreachable!(),
        };
        Ok(Built {
            processor,
            parameters,
        })
    }
}
#[derive(Default)]
struct Delay {
    data: Vec<f32>,
    cursor: usize,
    filled: usize,
}
impl Delay {
    fn new(frames: usize, total: &mut usize) -> Result<Self, Error> {
        *total = total
            .checked_add(frames.checked_mul(2).ok_or(Error::InvalidBuffer)?)
            .ok_or(Error::InvalidBuffer)?;
        if frames > 262144 || *total > 16 * 1024 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self {
            data: vec![0.; frames * 2],
            cursor: 0,
            filled: 0,
        })
    }
    fn process(&mut self, data: &mut [f32]) {
        if self.data.is_empty() {
            return;
        }
        for s in data {
            let old = if self.filled < self.data.len() {
                0.
            } else {
                self.data[self.cursor]
            };
            self.data[self.cursor] = *s;
            *s = old;
            self.cursor = (self.cursor + 1) % self.data.len();
            self.filled = (self.filled + 1).min(self.data.len());
        }
    }
    fn reset(&mut self) {
        self.cursor = 0;
        self.filled = 0;
    }
}
struct TrackRuntime {
    native_tap: Option<Arc<native_buses::Frames>>,
    monitor_pcm: Vec<f32>,
    sidechain: Option<Arc<ChannelBus>>,
    external_input: Option<(Arc<ChannelBus>, Vec<u16>)>,
    offline: bool,
    streamed: Option<StreamedClips>,
    player: Player,
    instrument: Option<Box<dyn AudioProcessor>>,
    inserts: Vec<Box<dyn AudioProcessor>>,
    input_gain: f32,
    midi_input: bool,
    source_delay: Delay,
    align: Delay,
    latency: usize,
    pcm: Vec<f32>,
    instrument_pcm: Vec<f32>,
    events: Vec<Event>,
    previous: Option<u64>,
    panic: bool,
}
impl TrackRuntime {
    fn process(
        &mut self,
        input: &[f32],
        ctx: &ProcessContext<'_>,
        input_offset: usize,
    ) -> Result<(), Error> {
        let len = input.len();
        let pcm = &mut self.pcm[..len];
        if self.previous != Some(ctx.transport.sample_position) {
            self.source_delay.reset();
            self.align.reset();
            self.panic = true;
        }
        self.player
            .render(&mut AudioBuffer::new(pcm, 2)?, ctx.transport)?;
        if let Some(streams) = &mut self.streamed {
            if self.offline {
                streams.mix_offline_into(pcm, ctx.transport)?;
            } else {
                streams.mix_into(pcm, ctx.transport)?;
            }
        }
        self.events.clear();
        if self.panic {
            self.events.push(Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            });
            self.panic = false;
        }
        let external = if self.midi_input { ctx.events } else { &[] };
        if self.events.len() + self.player.last_events().len() + external.len() > 1024 {
            return Err(Error::InvalidEvents);
        }
        self.events.extend_from_slice(self.player.last_events());
        self.events.extend_from_slice(external);
        for i in 1..self.events.len() {
            let mut j = i;
            while j > 0 && self.events[j].frame < self.events[j - 1].frame {
                self.events.swap(j, j - 1);
                j -= 1;
            }
        }
        if let Some((bus, channels)) = &self.external_input {
            for (f, pair) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                for c in 0..2 {
                    pair[c] += bus.sample(
                        input_offset + f,
                        channels[c.min(channels.len() - 1)] as usize,
                    ) * self.input_gain;
                }
            }
        } else {
            for (s, v) in pcm.iter_mut().zip(input) {
                *s += *v * self.input_gain;
            }
        }

        self.source_delay.process(pcm);
        let context = ProcessContext {
            events: &self.events,
            transport: ctx.transport,
        };
        if let Some(p) = &mut self.instrument {
            let output = &mut self.instrument_pcm[..len];
            output.fill(0.);
            p.process(&mut AudioBuffer::new(output, 2)?, &context);
            for (a, b) in pcm.iter_mut().zip(output) {
                *a += *b;
            }
        }
        for p in &mut self.inserts {
            p.process(&mut AudioBuffer::new(pcm, 2)?, &context);
        }
        if let Some(tap) = &self.native_tap {
            tap.write(ctx.transport.sample_position, pcm)?;
        }
        if let Some(bus) = &self.sidechain {
            bus.write(pcm)?;
        }
        if !self.monitor_pcm.is_empty() {
            self.monitor_pcm[..len].copy_from_slice(pcm);
        }
        self.align.process(pcm);
        self.previous = Some(
            ctx.transport.sample_position
                + if ctx.transport.playing {
                    (len / 2) as u64
                } else {
                    0
                },
        );
        Ok(())
    }
    fn reset(&mut self) {
        self.player.reset();
        if let Some(s) = &mut self.streamed {
            s.reset();
        }
        self.source_delay.reset();
        self.align.reset();
        self.previous = None;
        self.panic = true;
    }
}
struct AuxRuntime {
    inserts: Vec<Box<dyn AudioProcessor>>,
    gain: Arc<Parameter>,
    latency: usize,
    align: Delay,
}
struct Renderer {
    routed_taps: Vec<delivery::Tap>,
    cues: Option<cues::Runtime>,
    mix_routing: Option<mix_graph::Runtime>,
    track_order: Vec<usize>,
    post: Vec<Vec<f32>>,
    capture: Arc<CaptureStore>,
    tracks: Vec<TrackRuntime>,
    mixer: Mixer,
    bindings: Vec<(Arc<Controls>, Vec<Arc<Parameter>>)>,
    auxiliaries: Vec<AuxRuntime>,
    main_delay: Delay,
    sends: Vec<Vec<f32>>,
    input: Vec<f32>,
    spec: ProcessSpec,
    latency: usize,
    previous: Option<u64>,
    failures: Arc<AtomicU64>,
}
impl Renderer {
    fn render(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) -> Result<(), Error> {
        let cue_offset = self.capture.input_offset();
        let len = b.samples().len();
        self.input[..len].copy_from_slice(b.samples());
        if self.previous != Some(ctx.transport.sample_position) {
            self.main_delay.reset();
            if let Some(r) = &mut self.mix_routing {
                r.reset();
            }
            if let Some(c) = &mut self.cues {
                c.reset();
            }
            for a in &mut self.auxiliaries {
                a.align.reset();
            }
        }
        for &i in &self.track_order {
            self.tracks[i].process(&self.input[..len], ctx, self.capture.input_offset())?;
        }
        self.capture.write(&self.tracks, len);
        for (c, p) in &self.bindings {
            c.set_gain(p[0].get())?;
            c.set_pan(p[1].get())?;
            c.set_mute(p[2].get() >= 0.5);
            c.set_solo(p[3].get() >= 0.5);
            for (i, p) in p[4..].iter().enumerate() {
                c.set_send(i, p.get())?;
            }
        }
        let mut inputs: [&[f32]; 32] = [&[]; 32];
        for (i, t) in self.tracks.iter().enumerate() {
            inputs[i] = &t.pcm[..len];
        }
        let mut sends: [&mut [f32]; 4] = std::array::from_fn(|_| &mut [] as &mut [f32]);
        for (output, data) in sends.iter_mut().zip(self.sends.iter_mut()) {
            *output = &mut data[..len];
        }
        let mut post: [&mut [f32]; 32] = std::array::from_fn(|_| &mut [] as &mut [f32]);
        for (out, data) in post.iter_mut().zip(&mut self.post) {
            *out = &mut data[..len];
        }
        let audible = self
            .mix_routing
            .as_ref()
            .map_or([true; 32], |r| r.audible_tracks());
        self.mixer.process_routed(
            &inputs[..self.tracks.len()],
            b.samples_mut(),
            &mut sends[..self.auxiliaries.len()],
            &[],
            &mut post[..self.tracks.len()],
            &audible[..self.tracks.len()],
        )?;
        if self.mix_routing.is_none() {
            self.main_delay.process(b.samples_mut());
        }
        for (a, send) in self.auxiliaries.iter_mut().zip(&mut self.sends) {
            for p in &mut a.inserts {
                p.process(&mut AudioBuffer::new(&mut send[..len], 2)?, ctx);
            }
            a.align.process(&mut send[..len]);
            for s in &mut send[..len] {
                *s *= a.gain.get();
            }
            if self.mix_routing.is_none() {
                for (out, source) in b.samples_mut().iter_mut().zip(&send[..len]) {
                    *out += *source;
                }
            }
        }
        if let Some(r) = &mut self.mix_routing {
            r.process(&self.post, &self.sends, b.samples_mut(), ctx)?;
        }
        let track_count = self.post.len();
        let bus_count = self.routed_taps.len() - track_count - self.sends.len();
        for (i, tap) in self.routed_taps.iter().enumerate() {
            if !tap.enabled.load(Ordering::Relaxed) {
                continue;
            }
            let data = if i < track_count {
                &self.post[i][..len]
            } else if i < track_count + bus_count {
                self.mix_routing
                    .as_ref()
                    .ok_or(Error::InvalidRouting)?
                    .bus_pcm(i - track_count, len)
            } else {
                &self.sends[i - track_count - bus_count][..len]
            };
            tap.write(data, cue_offset)?;
        }
        if let Some(c) = &mut self.cues {
            c.process(
                &self.tracks,
                &self.post,
                self.mix_routing.as_ref(),
                &self.sends,
                b.samples(),
                cue_offset,
            )?;
        }
        self.previous = Some(
            ctx.transport.sample_position
                + if ctx.transport.playing {
                    b.frames() as u64
                } else {
                    0
                },
        );
        if b.samples().iter().any(|v| !v.is_finite() || v.abs() > 1e6) {
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
}
impl AudioProcessor for Renderer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if !same_spec(s, self.spec) {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self.render(b, ctx).is_err() {
            b.clear();
            self.failures.fetch_add(1, Ordering::Relaxed);
            self.reset();
        }
    }
    fn reset(&mut self) {
        for t in &mut self.tracks {
            t.reset();
        }
        for a in &mut self.auxiliaries {
            a.align.reset();
        }
        self.main_delay.reset();
        if let Some(r) = &mut self.mix_routing {
            r.reset();
        }
        if let Some(c) = &mut self.cues {
            c.reset();
        }
        self.previous = None;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
fn same_spec(a: ProcessSpec, b: ProcessSpec) -> bool {
    a.sample_rate == b.sample_rate
        && a.channels == b.channels
        && a.max_block_frames <= b.max_block_frames
}
#[derive(Default)]
pub struct State {
    playhead: AtomicU64,
    revision: AtomicU64,
    failures: AtomicU64,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Status {
    pub playhead: u64,
    pub applied_revision: u64,
    pub failures: u64,
}
impl State {
    pub fn snapshot(&self) -> Status {
        Status {
            playhead: self.playhead.load(Ordering::Relaxed),
            applied_revision: self.revision.load(Ordering::Acquire),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}
pub struct PreparedSession {
    routed_taps: Vec<delivery::Tap>,
    cue_click_delays: Vec<Delay>,
    click_bus: Option<Arc<ChannelBus>>,
    output_bus: Option<Arc<ChannelBus>>,
    cue_latencies: Vec<(String, usize)>,
    mix_meters: Vec<Arc<mix_graph::Meter>>,
    offline: bool,
    capture: Arc<CaptureStore>,
    streaming_stats: Vec<StreamTelemetry>,
    document: Document,
    inner: Box<dyn AudioProcessor>,
    clock: Clock,
    spec: ProcessSpec,
    latency: usize,
    parameters: Vec<Arc<Parameter>>,
    gestures: Vec<Gesture>,
    source_end: Option<u64>,
    scratch: Vec<Event>,
    state: Arc<State>,
    revision: u64,
    failures: Arc<AtomicU64>,
}
/// Offline-only ownership boundary. Rendering may wait for disk pages and must run off callback.
pub struct OfflineSession {
    inner: PreparedSession,
}
impl OfflineSession {
    pub fn routed_taps(&self) -> &[delivery::Tap] {
        &self.inner.routed_taps
    }
    pub fn render_block(
        &mut self,
        buffer: &mut AudioBuffer<'_>,
        context: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        if context.transport.sample_position > 1 << 48 {
            buffer.clear();
            return Err(Error::InvalidParameter);
        }
        let before = self.inner.state.failures.load(Ordering::Acquire);
        self.inner.process(buffer, context);
        if self.inner.state.failures.load(Ordering::Acquire) != before {
            return Err(Error::NotPrepared);
        }
        Ok(())
    }
    pub fn latency_frames(&self) -> usize {
        self.inner.latency
    }
}
impl PreparedSession {
    pub fn output_bus(&self) -> Option<&Arc<ChannelBus>> {
        self.output_bus.as_ref()
    }
    pub fn cue_latencies(&self) -> &[(String, usize)] {
        &self.cue_latencies
    }
    pub fn mix_meters(&self) -> &[Arc<mix_graph::Meter>] {
        &self.mix_meters
    }
    pub fn streaming_stats(&self) -> &[StreamTelemetry] {
        &self.streaming_stats
    }

    pub fn spec(&self) -> ProcessSpec {
        self.spec
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn gestures(&self) -> &[Gesture] {
        &self.gestures
    }
}
impl AudioProcessor for PreparedSession {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if !same_spec(s, self.spec) {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if let Some(bus) = &self.output_bus {
            if bus.offset() == 0 {
                bus.begin();
            }
        }
        if let Some(bus) = &self.click_bus {
            if bus.offset() == 0 {
                bus.begin();
            }
        }
        if b.channels() != 2
            || b.frames() > self.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            self.state.failures.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.source_end != Some(ctx.transport.sample_position)
            && self.clock.seek(ctx.transport.sample_position).is_err()
        {
            b.clear();
            return;
        }
        self.capture.begin();
        for t in &self.routed_taps {
            t.begin();
        }
        self.clock.set_playing(ctx.transport.playing);
        let mut offset = 0;
        let frames = b.frames();
        while offset < frames {
            let Ok(start) = self.clock.frame() else {
                b.clear();
                return;
            };
            let mut count = 1;
            if self.clock.advance(1).is_err() {
                b.clear();
                return;
            }
            while offset + count < frames {
                let Ok(next) = self.clock.frame() else {
                    b.clear();
                    return;
                };
                if next.sample != start.sample + if start.playing { count as u64 } else { 0 }
                    || next.position.bpm != start.position.bpm
                    || next.looped
                {
                    break;
                }
                count += 1;
                if self.clock.advance(1).is_err() {
                    b.clear();
                    return;
                }
            }
            let at_start = start.loop_seconds.is_some_and(|range| {
                (range[0] * self.spec.sample_rate).round() as u64 == start.sample
            });
            self.capture.clock(
                offset,
                count,
                start.sample,
                start.playing,
                start.looped,
                at_start,
            );
            self.scratch.clear();
            self.scratch.extend(
                ctx.events
                    .iter()
                    .filter(|e| e.frame >= offset && e.frame < offset + count)
                    .map(|e| Event {
                        frame: e.frame - offset,
                        ..*e
                    }),
            );
            self.inner.process(
                &mut AudioBuffer::new(&mut b.samples_mut()[offset * 2..(offset + count) * 2], 2)
                    .expect("stereo"),
                &ProcessContext {
                    events: &self.scratch,
                    transport: start.core_transport(),
                },
            );
            offset += count;
        }
        self.source_end = Some(
            ctx.transport.sample_position
                + if ctx.transport.playing {
                    frames as u64
                } else {
                    0
                },
        );
        if self.revision >= self.state.revision.load(Ordering::Acquire) {
            if let Ok(frame) = self.clock.frame() {
                self.state.playhead.store(frame.sample, Ordering::Relaxed);
            }
        }
        self.state
            .failures
            .fetch_add(self.failures.swap(0, Ordering::Relaxed), Ordering::Relaxed);
        self.state
            .revision
            .fetch_max(self.revision, Ordering::Release);
    }
    fn reset(&mut self) {
        let _ = self.clock.seek(0);
        self.source_end = None;
        self.inner.reset();
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
enum Command {
    Capture(Arc<CaptureStore>),
    Attach(u64, Box<RecordingBinding>),
    Detach(u64),
    Playing(bool),
    Seek(u64),
}
pub struct Controller {
    output_bus: Option<Arc<ChannelBus>>,
    cue_latencies: Vec<(String, usize)>,
    mix_meters: Vec<Arc<mix_graph::Meter>>,
    capture: Arc<CaptureStore>,
    recording_busy: Arc<AtomicBool>,
    midi_busy: Arc<AtomicBool>,
    recording_generation: u64,
    retired_recorders: rtrb::Consumer<Box<RecordingBinding>>,
    settling: bool,
    latency_budget: usize,
    streaming_stats: Vec<StreamTelemetry>,
    document: Document,
    commands: rtrb::Producer<Command>,
    graph: GraphController,
    spec: ProcessSpec,
    state: Arc<State>,
    parameters: Vec<Arc<Parameter>>,
    gestures: Vec<Gesture>,
    revision: u64,
}
pub struct SessionProcessor {
    cue_click_delays: Vec<Delay>,
    click_bus: Option<Arc<ChannelBus>>,
    cue_meters: Vec<Arc<mix_graph::Meter>>,
    output_bus: Option<Arc<ChannelBus>>,
    capture: Arc<CaptureStore>,
    recording: Option<Box<RecordingBinding>>,
    retired_recorders: rtrb::Producer<Box<RecordingBinding>>,
    detach: bool,
    recording_generation: u64,
    capture_clock: u64,
    commands: rtrb::Consumer<Command>,
    graph: LiveGraph,
    spec: ProcessSpec,
    state: Arc<State>,
    source: u64,
    playing: bool,
    dry: Vec<f32>,
    recorder: Option<soniclaw_record::RecordTap>,
}
pub fn live_pair(
    mut session: PreparedSession,
    latency_budget: usize,
    fade_frames: usize,
) -> Result<(Controller, SessionProcessor), Error> {
    if session.offline {
        return Err(Error::InvalidSpec);
    }
    let spec = session.spec;
    let state = Arc::new(State::default());
    session.state = state.clone();
    let capture = session.capture.clone();
    let recording_busy = Arc::new(AtomicBool::new(false));
    let (retire_recorders, retired_recorders) = rtrb::RingBuffer::new(2);
    let streaming_stats = session.streaming_stats.clone();
    let document = session.document.clone();
    let cue_click_delays = std::mem::take(&mut session.cue_click_delays);
    let click_bus = session.click_bus.clone();
    let cue_meters = session
        .mix_meters
        .iter()
        .filter(|m| m.id.starts_with("cue."))
        .cloned()
        .collect();
    let output_bus = session.output_bus.clone();
    let cue_latencies = session.cue_latencies.clone();
    let mix_meters = session.mix_meters.clone();
    let parameters = session.parameters.clone();
    let gestures = session.gestures.clone();
    let (graph, processor) = graph_pair(Box::new(session), spec, latency_budget, fade_frames)?;
    let (tx, rx) = rtrb::RingBuffer::new(32);
    Ok((
        Controller {
            output_bus: output_bus.clone(),
            cue_latencies,
            mix_meters,
            capture: capture.clone(),
            recording_busy,
            midi_busy: Arc::new(AtomicBool::new(false)),
            recording_generation: 0,
            retired_recorders,
            settling: false,
            latency_budget,
            streaming_stats,
            document,
            commands: tx,
            graph,
            spec,
            state: state.clone(),
            parameters,
            gestures,
            revision: 1,
        },
        SessionProcessor {
            cue_click_delays,
            click_bus,
            cue_meters,
            output_bus,
            capture,
            recording: None,
            retired_recorders: retire_recorders,
            detach: false,
            recording_generation: 0,
            capture_clock: 0,
            commands: rx,
            graph: processor,
            spec,
            state,
            source: 0,
            playing: false,
            dry: vec![0.; spec.max_block_frames * 2],
            recorder: None,
        },
    ))
}
impl Controller {
    pub fn output_bus(&self) -> Option<&Arc<ChannelBus>> {
        self.output_bus.as_ref()
    }
    pub fn cue_latencies(&self) -> &[(String, usize)] {
        &self.cue_latencies
    }
    pub fn mix_meters(&self) -> &[Arc<mix_graph::Meter>] {
        &self.mix_meters
    }
    pub fn recording_specs(
        &self,
        sources: &[(String, RecordingSource)],
    ) -> Result<Vec<soniclaw_record::multitrack::TrackSpec>, Error> {
        self.capture.specs(sources, self.latency_budget)
    }
    pub(crate) fn attach_multitrack_before_start(
        &mut self,
        tap: soniclaw_record::multitrack::Tap,
        sources: &[RecordingSource],
    ) -> Result<(), Error> {
        if self.status().applied_revision != 0
            || self.revision != 1
            || self.recording_generation != 0
            || self.midi_busy.load(Ordering::Acquire)
            || self.recording_busy.load(Ordering::Acquire)
            || self.commands.slots() == 0
        {
            return Err(Error::NotPrepared);
        }
        let binding =
            RecordingBinding::new(tap, sources, &self.capture, self.spec, self.latency_budget)?;
        self.commands
            .push(Command::Attach(1, Box::new(binding)))
            .map_err(|_| Error::NotPrepared)?;
        self.recording_generation = 1;
        self.recording_busy.store(true, Ordering::Release);
        Ok(())
    }
    pub fn attach_multitrack(
        &mut self,
        tap: soniclaw_record::multitrack::Tap,
        sources: &[RecordingSource],
    ) -> Result<(), Error> {
        if self.recording_generation == u64::MAX
            || self.midi_busy.load(Ordering::Acquire)
            || self.recording_busy.load(Ordering::Acquire)
            || self.settling
            || self.status().applied_revision != self.revision
            || self.commands.slots() == 0
        {
            return Err(Error::NotPrepared);
        }
        let binding =
            RecordingBinding::new(tap, sources, &self.capture, self.spec, self.latency_budget)?;
        self.commands
            .push(Command::Attach(
                self.recording_generation + 1,
                Box::new(binding),
            ))
            .map_err(|_| Error::InvalidRouting)?;
        self.recording_generation += 1;
        self.recording_busy.store(true, Ordering::Release);
        Ok(())
    }
    pub fn detach_multitrack(&mut self) -> Result<(), Error> {
        self.commands
            .push(Command::Detach(self.recording_generation))
            .map_err(|_| Error::InvalidRouting)
    }
    pub fn collect_recorders(&mut self) -> usize {
        let mut n = 0;
        while let Ok(tap) = self.retired_recorders.pop() {
            drop(tap);
            n += 1;
        }
        if n > 0 {
            self.recording_busy.store(false, Ordering::Release);
        }
        n
    }

    pub fn streaming_stats(&self) -> &[StreamTelemetry] {
        &self.streaming_stats
    }

    pub fn snapshot(&self) -> Result<Document, Error> {
        if self.state.snapshot().applied_revision != self.revision {
            return Err(Error::NotPrepared);
        }
        let mut d = self.document.clone();
        d.parameter_values = self
            .parameters
            .iter()
            .map(|p| (p.info().id.to_owned(), p.normalized()))
            .collect();
        d.validate()?;
        Ok(d)
    }

    pub fn set_playing(&mut self, playing: bool) -> Result<(), Error> {
        self.commands
            .push(Command::Playing(playing))
            .map_err(|_| Error::InvalidRouting)
    }
    pub fn seek(&mut self, sample: u64) -> Result<(), Error> {
        if sample > 1 << 48 {
            return Err(Error::InvalidParameter);
        }
        self.commands
            .push(Command::Seek(sample))
            .map_err(|_| Error::InvalidRouting)
    }
    pub fn status(&self) -> Status {
        self.state.snapshot()
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn gestures(&self) -> &[Gesture] {
        &self.gestures
    }
    pub fn collect_retired(&mut self) -> usize {
        self.collect_recorders();
        let n = self.graph.collect_retired();
        if n > 0 {
            self.settling = false;
        }
        n
    }
    pub fn replace(&mut self, mut session: PreparedSession) -> Result<u64, Error> {
        if self.output_bus.is_some() || session.output_bus.is_some() {
            return Err(Error::InvalidRouting);
        }
        if session.offline
            || !same_spec(session.spec, self.spec)
            || session.spec.max_block_frames != self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        if self.recording_busy.load(Ordering::Acquire)
            || self.midi_busy.load(Ordering::Acquire)
            || self.commands.slots() == 0
        {
            return Err(Error::InvalidRouting);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::InvalidParameter)?;
        session.revision = revision;
        session.state = self.state.clone();
        let streaming_stats = session.streaming_stats.clone();
        let document = session.document.clone();
        let mix_meters = session.mix_meters.clone();
        let parameters = session.parameters.clone();
        let gestures = session.gestures.clone();
        let capture = session.capture.clone();
        self.graph.replace(Box::new(session))?;
        self.commands
            .push(Command::Capture(capture.clone()))
            .map_err(|_| Error::InvalidRouting)?;
        self.capture = capture;
        self.settling = true;
        self.streaming_stats = streaming_stats;
        self.document = document;
        self.parameters = parameters;
        self.mix_meters = mix_meters;
        self.gestures = gestures;
        self.revision = revision;
        Ok(revision)
    }
}
impl SessionProcessor {
    pub fn attach_recorder(&mut self, tap: soniclaw_record::RecordTap) {
        self.recorder = Some(tap);
    }
    pub fn state(&self) -> Arc<State> {
        self.state.clone()
    }
}
impl AudioProcessor for SessionProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if !same_spec(s, self.spec) {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if let Some(bus) = &self.output_bus {
            if bus.offset() == 0 {
                bus.begin();
            }
        }
        if let Some(bus) = &self.click_bus {
            if bus.offset() == 0 {
                bus.begin();
            }
        }
        if b.channels() != 2
            || b.frames() > self.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
            || b.samples().iter().any(|v| !v.is_finite() || v.abs() > 1e6)
        {
            b.clear();
            self.state.failures.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for _ in 0..32 {
            match self.commands.pop() {
                Ok(Command::Capture(capture)) => self.capture = capture,
                Ok(Command::Attach(generation, binding)) => {
                    self.recording_generation = generation;
                    self.detach = false;
                    self.capture.enabled.store(true, Ordering::Release);
                    self.recording = Some(binding);
                }
                Ok(Command::Detach(generation)) => {
                    if generation == self.recording_generation {
                        self.detach = true
                    }
                }
                Ok(Command::Playing(p)) => self.playing = p,
                Ok(Command::Seek(s)) => {
                    self.source = s;
                    self.graph.reset();
                }
                Err(_) => break,
            }
        }
        if self.detach && self.retired_recorders.slots() > 0 {
            if let Some(binding) = self.recording.take() {
                if let Err(rtrb::PushError::Full(binding)) = self.retired_recorders.push(binding) {
                    self.recording = Some(binding);
                } else {
                    self.capture.enabled.store(false, Ordering::Release);
                    self.detach = false;
                }
            } else {
                self.detach = false;
            }
        }
        let len = b.samples().len();
        self.dry[..len].copy_from_slice(b.samples());
        self.graph.process(
            b,
            &ProcessContext {
                events: ctx.events,
                transport: Transport {
                    sample_position: self.source,
                    tempo: 120.,
                    playing: self.playing,
                },
            },
        );
        if let Some(binding) = &mut self.recording {
            if binding
                .capture(
                    &self.capture,
                    self.capture_clock,
                    &self.dry[..len],
                    b.samples(),
                )
                .is_err()
            {
                self.state.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.capture_clock = self.capture_clock.saturating_add(b.frames() as u64);
        if let Some(tap) = &mut self.recorder {
            tap.capture_pair(&self.dry[..len], b.samples());
        }
        if self.playing {
            self.source = self.source.saturating_add(b.frames() as u64);
        }
    }
    fn reset(&mut self) {
        for d in &mut self.cue_click_delays {
            d.reset();
        }
        self.source = 0;
        self.graph.reset();
    }
    fn latency_frames(&self) -> usize {
        self.graph.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_clips::AudioAsset;
    pub(super) fn fixture() -> (Document, Assets) {
        let d = Document {
            cues: Vec::new(),
            mix_routing: Default::default(),
            parameter_values: BTreeMap::new(),
            schema: 1,
            sample_rate: 48000,
            max_block_frames: 256,
            timeline: Default::default(),
            tracks: vec![Track {
                streamed: vec![],
                id: "audio".into(),
                audio: vec![AudioClip {
                    edits: None,
                    id: "loop".into(),
                    asset: "a".into(),
                    track: 0,
                    start_frame: 0,
                    length_frames: 96000,
                    source_offset: 0,
                    source_length: 4,
                    looping: true,
                    gain: 1.,
                    fade_in: 0,
                    fade_out: 0,
                    enabled: true,
                }],
                midi: vec![],
                instrument: None,
                inserts: vec![Descriptor {
                    id: "audio.fx".into(),
                    kind: "gain".into(),
                    state: serde_json::json!({"value":0.5}),
                }],
                input_gain: 0.,
                midi_input: false,
            }],
            mixer: MixerModel {
                schema: 1,
                strips: vec![soniclaw_mixer::Strip {
                    id: "audio".into(),
                    name: "Audio".into(),
                    gain: 1.,
                    pan: 0.,
                    mute: false,
                    solo: false,
                    group: None,
                    sends: vec![],
                }],
                groups: vec![],
                taps: vec![],
            },
            auxiliaries: vec![],
            automation: Automation::default(),
        };
        (
            d,
            Assets::from([(
                "a".into(),
                AudioAsset {
                    sample_rate: 48000,
                    channels: 1,
                    samples: Arc::from([0.2; 4]),
                },
            )]),
        )
    }
    fn rendered(d: &Document, assets: &Assets, block: usize, frames: usize) -> Vec<f32> {
        let p = d.compile(assets, &mut Builtins).unwrap();
        let (mut c, mut p) = live_pair(p, 0, 32).unwrap();
        c.set_playing(true).unwrap();
        let mut data = vec![0.; frames * 2];
        let alloc = allocation_counter::measure(|| {
            for pcm in data.chunks_mut(block * 2) {
                p.process(
                    &mut AudioBuffer::new(pcm, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Default::default(),
                    },
                );
            }
        });
        assert_eq!(alloc.count_total, 0);
        assert_eq!(c.status().failures, 0);
        data
    }
    #[test]
    fn live_clips_inserts_mixer_clock_loop_and_partition() {
        let (mut d, a) = fixture();
        let pcm = rendered(&d, &a, 64, 2000);
        assert!((pcm[0] - 0.1 * 0.5f32.sqrt()).abs() < 1e-6);
        d.timeline.loop_range = Some([0., 0.001]);
        let p = d.compile(&a, &mut Builtins).unwrap();
        let (mut c, mut p) = live_pair(p, 0, 32).unwrap();
        c.set_playing(true).unwrap();
        let mut data = [0.; 128];
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        assert_eq!(c.status().playhead, 16);
        assert!(data.iter().all(|s| (*s - pcm[0]).abs() < 1e-6));
        d.timeline.loop_range = None;
        d.automation.lanes.push(soniclaw_automation::Lane {
            parameter: "mix.audio.gain".into(),
            enabled: true,
            mode: soniclaw_automation::Mode::Read,
            points: vec![
                soniclaw_automation::Point {
                    beat: 0.,
                    value: 0.25,
                    curve: soniclaw_automation::Curve::Linear,
                },
                soniclaw_automation::Point {
                    beat: 0.08,
                    value: 0.,
                    curve: soniclaw_automation::Curve::Linear,
                },
            ],
        });
        let one = rendered(&d, &a, 64, 4000);
        let two = rendered(&d, &a, 127, 4000);
        assert_eq!(one, two);
        assert!(one[7998] < one[0] * 0.2);
    }
    #[test]
    fn midi_while_stopped_and_invalid_input() {
        let (mut d, a) = fixture();
        d.tracks[0].audio.clear();
        d.tracks[0].inserts.clear();
        d.tracks[0].instrument = Some(Descriptor {
            id: "lead".into(),
            kind: "sine".into(),
            state: serde_json::json!({}),
        });
        d.tracks[0].midi_input = true;
        let p = d.compile(&a, &mut Builtins).unwrap();
        let (c, mut p) = live_pair(p, 0, 32).unwrap();
        let mut data = [0.; 128];
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[Event {
                    frame: 7,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 69,
                        velocity: 100,
                    },
                }],
                transport: Default::default(),
            },
        );
        assert!(data[..14].iter().all(|v| *v == 0.));
        assert!(data[14..].iter().any(|v| v.abs() > 1e-5));
        data[3] = f32::NAN;
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        assert!(data.iter().all(|v| *v == 0.));
        assert!(c.status().failures > 0);
    }
    #[test]
    fn replacement_is_atomic_and_old_processors_retire_off_callback() {
        use std::sync::atomic::AtomicUsize;
        struct Spy(Arc<AtomicUsize>);
        impl Drop for Spy {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        impl AudioProcessor for Spy {
            fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
                Ok(())
            }
            fn process(&mut self, _: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {}
            fn reset(&mut self) {}
        }
        let (d, a) = fixture();
        let drops = Arc::new(AtomicUsize::new(0));
        let mut factory = |_: &Descriptor| {
            Ok(Built {
                processor: Box::new(Spy(drops.clone())),
                parameters: vec![],
            })
        };
        let initial = d.compile(&a, &mut factory).unwrap();
        let (mut c, mut p) = live_pair(initial, 0, 16).unwrap();
        c.set_playing(true).unwrap();
        let replacement = d.compile(&a, &mut factory).unwrap();
        assert_eq!(c.replace(replacement).unwrap(), 2);
        let mut data = [0.; 128];
        let alloc = allocation_counter::measure(|| {
            for _ in 0..4 {
                p.process(
                    &mut AudioBuffer::new(&mut data, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Default::default(),
                    },
                );
            }
        });
        assert_eq!(alloc.count_total, 0);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(c.status().applied_revision, 2);
        assert_eq!(c.collect_retired(), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn document_roundtrip_factory_failure_and_record_tap() {
        let (d, a) = fixture();
        let copy = Document::from_json(&d.to_json().unwrap()).unwrap();
        let mut bad = copy.clone();
        bad.tracks[0].inserts[0].kind = "unknown".into();
        assert!(bad.compile(&a, &mut Builtins).is_err());
        let prepared = copy.compile(&a, &mut Builtins).unwrap();
        let (mut control, mut p) = live_pair(prepared, 0, 32).unwrap();
        let path = std::env::temp_dir().join(format!(
            "studio-take-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (mut recorder, tap) = soniclaw_record::Recorder::new(16);
        recorder
            .start(&path, 48000, soniclaw_record::RecordSource::Processed)
            .unwrap();
        p.attach_recorder(tap);
        control.set_playing(true).unwrap();
        let mut pcm = [0.; 128];
        for _ in 0..4 {
            p.process(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Default::default(),
                },
            );
        }
        recorder.stop().unwrap();
        assert_eq!(recorder.summary().written_frames, 256);
        assert_eq!(recorder.summary().dropped_frames, 0);
        assert!(std::fs::metadata(&path).unwrap().len() > 44);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod multitrack_integration {
    use super::*;
    #[test]
    fn captures_complete_tracks_and_blocks_replacement_until_control_retirement() {
        let (d, assets) = super::tests::fixture();
        let prepared = d.compile(&assets, &mut Builtins).unwrap();
        let (mut c, mut p) = live_pair(prepared, 0, 16).unwrap();
        let mut block = [0.3; 128];
        let context = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        p.process(&mut AudioBuffer::new(&mut block, 2).unwrap(), &context);
        let sources = vec![
            ("stem".into(), RecordingSource::Track("audio".into())),
            ("mix".into(), RecordingSource::Master),
            ("input".into(), RecordingSource::Input),
        ];
        let specs = c.recording_specs(&sources).unwrap();
        let (mut recorder, tap) =
            soniclaw_record::multitrack::pair(soniclaw_record::multitrack::Config {
                sample_rate: 48000,
                tracks: specs,
                pre_roll_frames: 64,
                queue_packets: 16,
                max_block_frames: 256,
            })
            .unwrap();
        c.attach_multitrack(
            tap,
            &sources.iter().map(|s| s.1.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
        c.set_playing(true).unwrap();
        for _ in 0..3 {
            block.fill(0.3);
            p.process(&mut AudioBuffer::new(&mut block, 2).unwrap(), &context);
        }
        let path = std::env::temp_dir().join(format!(
            "studio-multi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        recorder
            .start(
                &path,
                soniclaw_record::multitrack::TakePlan {
                    id: "one".into(),
                    label: "Studio".into(),
                    punch: None,
                },
            )
            .unwrap();
        assert!(c
            .replace(d.compile(&assets, &mut Builtins).unwrap())
            .is_err());
        let alloc = allocation_counter::measure(|| {
            for _ in 0..3 {
                block.fill(0.3);
                p.process(&mut AudioBuffer::new(&mut block, 2).unwrap(), &context);
            }
        });
        assert_eq!(alloc.count_total, 0);
        let take = recorder.stop().unwrap();
        assert_eq!(take.frames, 256);
        assert_eq!(take.tracks.len(), 3);
        assert!(take.complete);
        assert_eq!(take.missing_frames, 0);
        assert_eq!(c.status().failures, 0);
        c.detach_multitrack().unwrap();
        p.process(&mut AudioBuffer::new(&mut block, 2).unwrap(), &context);
        assert_eq!(c.collect_recorders(), 1);
        assert!(c
            .replace(d.compile(&assets, &mut Builtins).unwrap())
            .is_ok());
        let loaded = soniclaw_record::multitrack::Take::read(&path).unwrap();
        assert_eq!(loaded.tracks[0].peak, 0.1);
        assert!((loaded.tracks[1].peak - 0.1 * 0.5f32.sqrt()).abs() < 1e-6);
        assert_eq!(loaded.tracks[2].peak, 0.3);
        drop(p);
        drop(c);
        drop(recorder);
        std::fs::remove_dir_all(path).unwrap();
    }
}
#[cfg(test)]
mod loop_capture_tests {
    use super::*;
    #[test]
    fn actual_clock_wraps_become_sample_exact_take_passes() {
        let (mut d, a) = crate::tests::fixture();
        d.timeline.loop_range = Some([0., 0.01]);
        let (mut c, mut p) = live_pair(d.compile(&a, &mut Builtins).unwrap(), 0, 16).unwrap();
        let mut pcm = [0.; 254];
        let ctx = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        p.process(&mut AudioBuffer::new(&mut pcm, 2).unwrap(), &ctx);
        let sources = vec![("stem".into(), RecordingSource::Track("audio".into()))];
        let (mut recorder, tap) =
            soniclaw_record::multitrack::pair(soniclaw_record::multitrack::Config {
                sample_rate: 48000,
                tracks: c.recording_specs(&sources).unwrap(),
                pre_roll_frames: 0,
                queue_packets: 64,
                max_block_frames: 256,
            })
            .unwrap();
        c.attach_multitrack(tap, &[RecordingSource::Track("audio".into())])
            .unwrap();
        let path = std::env::temp_dir().join(format!("studio-loop54-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        recorder
            .start(
                &path,
                soniclaw_record::multitrack::TakePlan {
                    id: "loop".into(),
                    label: "clock".into(),
                    punch: None,
                },
            )
            .unwrap();
        c.set_playing(true).unwrap();
        let alloc = allocation_counter::measure(|| {
            for _ in 0..8 {
                p.process(&mut AudioBuffer::new(&mut pcm, 2).unwrap(), &ctx);
            }
        });
        assert_eq!(alloc.count_total, 0);
        let take = recorder.stop().unwrap();
        assert_eq!(
            take.loop_boundaries
                .iter()
                .map(|b| b.sample)
                .collect::<Vec<_>>(),
            vec![0, 240, 480, 720, 960]
        );
        assert!(take
            .loop_boundaries
            .iter()
            .all(|b| b.source_frame == 0
                && b.kind == soniclaw_record::multitrack::BoundaryKind::Loop));
        assert_eq!(
            soniclaw_record::comp::Recipe::new(&take)
                .unwrap()
                .passes
                .len(),
            4
        );
        drop(p);
        drop(c);
        drop(recorder);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(test)]
mod delivery_tests;
