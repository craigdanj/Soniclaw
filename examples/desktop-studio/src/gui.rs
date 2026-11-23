use soniclaw::core::ChannelBus;
use soniclaw::{
    device::{DeviceManager, DevicePreferences, NativeOutputBackend, OutputEngine},
    prelude::*,
    studio::{project::Project, *},
    ui::{self, egui},
};
use soniclaw_host::studio::Owner as Native;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
struct Factory<'a> {
    buses: native_buses::Registry,
    mode: soniclaw_host::studio::Mode,
    spec: ProcessSpec,
    assets: soniclaw::clips::Assets,
    states: soniclaw::studio::native_state::States,
    timeline: soniclaw::transport::Timeline,
    trusted: &'a BTreeSet<PathBuf>,
    owners: &'a mut Vec<Native>,
}
impl soniclaw::studio::Factory for Factory<'_> {
    fn prepare_document(&mut self, d: &Document) -> std::result::Result<(), Error> {
        self.buses.prepare(d)
    }
    fn track_tap(&self, id: &str) -> Option<std::sync::Arc<native_buses::Frames>> {
        self.buses.tracks.get(id).cloned()
    }

    fn create(&mut self, d: &Descriptor) -> std::result::Result<Built, Error> {
        if !matches!(d.kind.as_str(), "clap" | "vst3") || d.state["bypass"].as_bool() == Some(true)
        {
            let helpers = soniclaw_host::studio::isolation_adapter::TrustedHelpers::default();
            let loader = soniclaw_host::formats::TrustedLoader::default();
            return soniclaw_host::studio::NativeFactory {
                helpers: &helpers,
                bus_graph: &mut self.buses,
                mode: self.mode,
                spec: self.spec,
                timeline: &self.timeline,
                assets: &self.assets,
                states: &self.states,
                loader: &loader,
                owners: self.owners,
                last_error: None,
            }
            .create(d);
        }
        let result = (|| -> Result<Built> {
            let path = PathBuf::from(d.state["path"].as_str().ok_or("missing native path")?)
                .canonicalize()?;
            if !self.trusted.contains(&path) {
                return Err(
                    "native module requires explicit trust in plug-in manager or startup allowlist"
                        .into(),
                );
            }
            // SAFETY: user explicitly approved this canonical native module and its dependencies.
            let loader = unsafe { soniclaw_host::formats::TrustedLoader::new(&[path]) }?;
            // SAFETY: paths enter this session set only after explicit native-code approval.
            let helpers = unsafe {
                soniclaw_host::studio::isolation_adapter::TrustedHelpers::new(
                    &self.trusted.iter().cloned().collect::<Vec<_>>(),
                )
            }?;
            soniclaw_host::studio::NativeFactory {
                helpers: &helpers,
                bus_graph: &mut self.buses,
                mode: self.mode,
                spec: self.spec,
                timeline: &self.timeline,
                assets: &self.assets,
                states: &self.states,
                loader: &loader,
                owners: self.owners,
                last_error: None,
            }
            .create_detailed(d)
        })();
        result.map_err(|e| {
            eprintln!("native {}: {e}", d.id);
            Error::InvalidParameter
        })
    }
}
struct CompDraft {
    take: usize,
    manifest: soniclaw::record::multitrack::Take,
    recipe: soniclaw::record::comp::Recipe,
    stem: String,
}
type MidiConnection = soniclaw::midi::native::Connection;
type MidiInputs = (
    Option<MidiConnection>,
    soniclaw::device::EventSender,
    Vec<soniclaw::device::EventReceiver>,
);
fn midi_inputs(
    port: Option<usize>,
    telemetry: std::sync::Arc<soniclaw::device::Telemetry>,
) -> Result<MidiInputs> {
    let (keyboard, keyboard_rx) = soniclaw::device::event_channel(256, telemetry.clone());
    let mut receivers = vec![keyboard_rx];
    let connection = if let Some(port) = port {
        let timing = telemetry.clone();
        let mut mapper: Option<soniclaw::midi::schedule::TimestampMapper> = None;
        let (mut sender, receiver) = soniclaw::device::event_channel(2048, telemetry);
        let connection =
            soniclaw::midi::native::connect_input_timed(port, move |timestamp, message| {
                if mapper.is_none() {
                    if let Some((sample, rate)) = timing.input_clock.now() {
                        mapper = soniclaw::midi::schedule::TimestampMapper::new(
                            timestamp,
                            sample,
                            rate as f64,
                        )
                        .ok();
                    }
                }
                if let Some(sample) = mapper.as_ref().and_then(|m| m.sample(timestamp).ok()) {
                    sender.send_at(sample, message);
                } else {
                    sender.send_timed(message);
                }
            })
            .map_err(|e| e.to_string())?;
        receivers.push(receiver);
        Some(connection)
    } else {
        None
    };
    Ok((connection, keyboard, receivers))
}
struct SaveResult {
    baseline: Project,
    generation: u64,
}
struct Studio {
    isolation_tools: IsolationTools,
    native_buses: NativeBusTools,
    native_gestures: native_edits::Gestures,
    native_tools: NativeTools,
    tools: Tools,
    elastic: ElasticTools,
    mix: MixTools,
    cues: CueTools,

    play_from_beat: f64,
    timing_command: Option<arrangement::Command>,
    timing_pending: Option<soniclaw::transport::Timeline>,
    audio_anchor_beats: bool,
    tempo_beat: f64,
    tempo_bpm: f64,
    tempo_ramp: bool,
    meter_beat: f64,
    meter_numerator: u16,
    meter_denominator: u16,
    label_id: String,
    label_name: String,
    label_start: f64,
    label_end: f64,
    duplicate_beat: f64,
    duplicate_automation: bool,
    control_id: u32,
    control_beat: f64,
    control_channel: u8,
    control_kind: u8,
    control_key: u8,
    control_value: f32,
    midi_take_index: usize,
    midi_pass_index: usize,
    group_toggle: Option<(String, bool)>,
    ungroup_id: Option<String>,
    salvage_source: String,
    salvage_target: String,
    salvage_job: Option<jobs::Ticket<(soniclaw::record::salvage::Report, PathBuf)>>,
    latency_reference: String,
    latency_return: String,
    latency_job: Option<jobs::Ticket<timing::Measurement>>,
    save_job: Option<jobs::Ticket<SaveResult>>,
    saved_baseline: Option<Project>,
    pending_job: Option<jobs::Ticket<u64>>,
    pending_at: Instant,
    workspace_command: Option<workspace::Command>,
    new_project_open: bool,
    new_project_name: String,
    new_project_rate: u32,
    new_project_block: usize,
    track_name: String,
    track_folder_name: String,
    midi_clip: String,
    midi_connection: Option<MidiConnection>,
    midi_keyboard: Option<soniclaw::device::EventSender>,
    midi_held: std::collections::BTreeMap<u8, u8>,
    midi_ports: Vec<String>,
    midi_port: Option<usize>,
    midi_settings: midi_recording::Settings,
    midi_recorder: Option<midi_recording::Recorder>,
    midi_last: Option<midi_recording::Take>,
    midi_folder: String,
    midi_status: String,
    media_plan: Option<media::Plan>,
    media_audit_job: Option<jobs::Ticket<media::Plan>>,
    media_copy_job: Option<jobs::Ticket<media::Consolidated>>,
    media_folder: String,
    media_status: String,
    automation_writer: Option<automation_write::Writer>,
    automation_parameter: String,
    automation_mode: soniclaw::automation::Mode,
    automation_grid: f64,
    plugin_windows: soniclaw_host::editor_workspace::Workspace,
    routing_draft: Option<routing::Draft>,
    export_settings: export::Settings,
    export_folder: String,
    export_job: Option<jobs::Ticket<export::Report>>,
    export_report: String,
    jobs: jobs::Scheduler,
    job_epoch: u64,
    wave_epoch: u64,
    wave_editor: ui::wave_edit::Editor,
    wave_loader: Option<jobs::Ticket<soniclaw::waveform::Loaded>>,
    wave_thumbnail: Option<std::sync::Arc<soniclaw::waveform::Thumbnail>>,
    wave_draft: Option<wave_edit::Draft>,
    wave_job: Option<jobs::Ticket<wave_edit::Ready>>,
    wave_ready: Option<wave_edit::Ready>,
    wave_source: String,
    wave_import: String,
    record_settings: recording::Settings,
    record_progress: Option<std::sync::Arc<recording::Progress>>,
    input_enabled: bool,
    input_bus: Option<std::sync::Arc<ChannelBus>>,
    input_channels: [u16; 2],
    autosave: Option<recovery::Autosave>,
    autosave_enabled: bool,
    recovery_store: Option<recovery::Store>,
    recovery_folder: String,
    autosave_seconds: u64,
    resource_issues: Vec<soniclaw::studio::project::ResourceIssue>,
    repair_selected: usize,
    repair_path: String,
    history: history::History,
    freeze_job: Option<(jobs::Ticket<freeze::Frozen>, freeze::Monitor)>,
    project: Project,
    manager: DeviceManager,
    control: Option<Controller>,
    owners: Vec<Native>,
    trusted: BTreeSet<PathBuf>,
    recorder: Option<soniclaw::record::multitrack::Recorder>,
    recording: bool,
    record_path: Option<PathBuf>,
    selected: usize,
    editor: ui::timeline::TimelineEditor,
    comp_editor: ui::comp::CompEditor,
    comp: Option<CompDraft>,
    comp_target: String,

    automation: soniclaw::automation::ui::Editor,
    folder: String,
    plugin_path: String,
    plugin_id: String,
    status: String,
    serial: u64,
    started: Instant,
}
fn load_plugin_windows(root: &Path) -> Result<soniclaw_host::editor_workspace::Workspace> {
    let path = root.join("plugin-windows.json");
    if !path.exists() {
        return Ok(Default::default());
    }
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(262145)
        .read_to_string(&mut text)?;
    soniclaw_host::editor_workspace::Workspace::from_json(&text).map_err(|e| e.to_string().into())
}
impl Studio {
    fn fresh_id(&mut self, prefix: &str) -> String {
        loop {
            self.serial += 1;
            let id = format!("{prefix}-{}", self.serial);
            if !self.project.layout.folders.contains_key(&id)
                && !self.project.document.tracks.iter().any(|t| {
                    t.id == id
                        || t.midi.iter().any(|c| c.id == id)
                        || t.audio.iter().any(|c| c.id == id)
                        || t.streamed.iter().any(|c| c.id == id)
                })
            {
                return id;
            }
        }
    }
    fn sync_controls(&mut self) {
        if let Some(c) = &self.control {
            if let Ok(snapshot) = c.snapshot() {
                self.project.document.parameter_values = snapshot.parameter_values;
            }
        }
    }
    fn stop(&mut self) -> Result<()> {
        self.sync_controls();
        self.tools.preview_done = None;
        self.manager.stop();
        self.finish_recording()?;
        self.midi_connection = None;
        if let Some(sender) = &self.midi_keyboard {
            sender.all_notes_off();
        }
        self.midi_keyboard = None;
        self.midi_held.clear();
        self.manager.stop();
        self.service_native_edits(true)?;
        self.control = None;
        let mut automation_error = None;
        if let Some(mut writer) = self.automation_writer.take() {
            if let Err(e) = writer.commit(&mut self.project) {
                self.automation_writer = Some(writer);
                automation_error = Some(e);
            }
        }
        for native in &mut self.owners {
            if let Ok(state) = self.plugin_windows.for_instance(&native.id, &native.entry) {
                if let Some(size) = native.editor_size(0) {
                    state.size = Some(size);
                }
            }
            native.close_editor(0);
            let snapshot = native.snapshot()?;
            for d in native_state::descriptors_mut(&mut self.project.document) {
                if d.id == native.id {
                    d.state
                        .as_object_mut()
                        .ok_or("native state must be object")?
                        .remove("session");
                    if let Some(e) = &native.fault {
                        d.state["quarantined"] = serde_json::json!(true);
                        d.state["last_native_error"] =
                            serde_json::json!(e.chars().take(1024).collect::<String>());
                    }
                    d.state["native_state"] = serde_json::json!(true);
                    d.state["latency_frames"] = serde_json::json!(native.latency_frames());
                }
            }
            self.project
                .native_states
                .insert(native.id.clone(), snapshot);
            for c in &native.controls {
                self.project
                    .document
                    .parameter_values
                    .insert(c.parameter.info().id.into(), c.parameter.normalized());
            }
        }
        self.owners.clear();
        self.native_gestures
            .finish(&mut self.project, &mut self.history)?;
        self.recorder = None;
        self.finish_midi_take()?;
        if let Some(error) = automation_error {
            return Err(error);
        }
        Ok(())
    }
    fn play(&mut self) -> Result<()> {
        self.stop()?;
        self.configure_input()?;
        let mut beat = self.play_from_beat;
        if let Some([a, b]) = self.project.document.timeline.loop_range {
            if beat < a || beat >= b {
                beat = a;
            }
        }
        let start = soniclaw::transport::TempoMap::compile(&self.project.document.timeline)?
            .sample_at(beat, self.project.document.sample_rate as f64)?;
        let mut monitored = self.project.document.clone();
        for t in &mut monitored.tracks {
            t.input_gain = if self.input_enabled
                && self
                    .record_settings
                    .arms
                    .iter()
                    .any(|a| a.track == t.id && a.monitor_input)
            {
                1.
            } else {
                0.
            };
        }
        let input_bus = self.input_bus.clone();
        let routes = self
            .record_settings
            .arms
            .iter()
            .filter(|a| a.monitor_input && !a.channels.is_empty())
            .map(|a| (a.track.clone(), a.channels.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let d = &monitored;
        let assets = &self.project.assets;
        let disk = &self.project.disk;
        let trust = &self.trusted;
        let native_states = self.project.native_states.clone();
        let owners = &mut self.owners;
        let control = &mut self.control;
        let port = self.midi_port;
        let mut midi = None;
        self.manager
            .start(self.started.elapsed(), &mut |plan, _telemetry| {
                if plan.spec().sample_rate != d.sample_rate as f64
                    || plan.spec().channels != 2
                    || plan.spec().max_block_frames != d.max_block_frames
                {
                    return Err(
                        "device must support the project's stereo rate/block specification".into(),
                    );
                }
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Live,
                    spec: plan.spec(),
                    assets: assets.clone(),
                    states: native_states.clone(),
                    timeline: d.timeline.clone(),
                    trusted: trust,
                    owners,
                };
                let (connection, keyboard, receivers) =
                    midi_inputs(port, _telemetry).map_err(|e| e.to_string())?;
                midi = Some((connection, keyboard));
                let prepared = if let Some(bus) = input_bus.clone() {
                    d.compile_inputs_at(
                        assets,
                        disk,
                        &mut factory,
                        start,
                        &soniclaw::studio::inputs::Setup {
                            bus,
                            routes: routes.clone(),
                        },
                    )?
                } else {
                    d.compile_streamed_at(assets, disk, &mut factory, start)?
                };
                let budget = prepared.latency_frames();
                let (mut c, p) = live_pair(prepared, budget, 128)?;
                c.seek(start)?;
                c.set_playing(true)?;
                let output_bus = c.output_bus().cloned();
                *control = Some(c);
                let mut engine = Engine::default();
                engine.push(p);
                Ok(OutputEngine {
                    output_bus,

                    input_bus: input_bus.clone(),
                    engine,
                    receivers,
                })
            })
            .map_err(|e| e.to_string())?;
        if let Some((connection, keyboard)) = midi {
            self.midi_connection = connection;
            self.midi_keyboard = Some(keyboard);
        }
        if self.input_enabled {
            self.manager.set_monitor(true).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn configure_input(&mut self) -> Result<()> {
        let mut prefs = self.manager.preferences().clone();
        prefs.sample_rate = Some(self.project.document.sample_rate);
        prefs.max_internal_frames = self.project.document.max_block_frames;
        let mut route = prefs.duplex.clone().unwrap_or_default();
        route.input_count = Some(
            self.record_settings
                .arms
                .iter()
                .flat_map(|a| a.channels.iter())
                .copied()
                .chain(self.input_channels)
                .chain(cues::input_channels(&self.project.document))
                .max()
                .unwrap_or(1)
                + 1,
        );
        route.input_channels = self.input_channels;
        route.cue_outputs = self
            .project
            .document
            .cues
            .iter()
            .map(|c| c.outputs)
            .collect();
        if !self.input_enabled {
            route.input = None;
            route.use_default_input = false;
        } else if route.input.is_none() {
            route.use_default_input = true;
        }
        prefs.duplex = if self.input_enabled || !self.project.document.cues.is_empty() {
            Some(route)
        } else {
            None
        };
        if !self.project.document.cues.is_empty() {
            prefs.channels = None;
        }

        self.input_bus = if self.input_enabled {
            Some(std::sync::Arc::new(ChannelBus::new(
                prefs
                    .duplex
                    .as_ref()
                    .and_then(|d| d.input_count)
                    .unwrap_or(2) as usize,
                self.project.document.max_block_frames,
            )?))
        } else {
            None
        };
        self.manager = DeviceManager::new(NativeOutputBackend, prefs).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn begin_midi_take(&mut self) -> Result<()> {
        if self.recording || self.automation_writer.is_some() || self.midi_recorder.is_some() {
            return Err("finish the active recording/write take first".into());
        }
        self.stop()?;
        self.serial += 1;
        self.midi_settings.clip = workspace::identity("midi")?;
        if self.midi_settings.looping {
            self.midi_settings.start_beat = self
                .project
                .document
                .timeline
                .loop_range
                .as_ref()
                .ok_or("enable an arrangement loop first")?[0];
        }
        self.midi_settings.validate(&self.project)?;
        if self.midi_folder.is_empty() || Path::new(&self.midi_folder).exists() {
            loop {
                self.midi_folder = format!("{}-midi-{}", self.folder, self.serial);
                if !Path::new(&self.midi_folder).exists() {
                    break;
                }
                self.serial += 1;
            }
        }
        if Path::new(&self.midi_folder).exists() {
            return Err("MIDI take folder must be new".into());
        }
        self.configure_input()?;
        let input_bus = self.input_bus.clone();
        let project = &self.project;
        let mut settings = self.midi_settings.clone();
        settings.input_offset_frames = self
            .project
            .layout
            .calibration
            .as_ref()
            .map_or(0, |c| c.midi_offset_frames)
            + if self.midi_port.is_some() {
                self.project.document.max_block_frames as i32
            } else {
                0
            };
        let port = self.midi_port;
        let trust = &self.trusted;
        let owners = &mut self.owners;
        let mut captured = None;
        self.manager
            .start(self.started.elapsed(), &mut |plan, telemetry| {
                if plan.spec().sample_rate != project.document.sample_rate as f64
                    || plan.spec().channels != 2
                    || plan.spec().max_block_frames != project.document.max_block_frames
                {
                    return Err("device must match project specification".into());
                }
                let (connection, keyboard, receivers) =
                    midi_inputs(port, telemetry).map_err(|e| e.to_string())?;
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Live,
                    spec: plan.spec(),
                    assets: project.assets.clone(),
                    states: project.native_states.clone(),
                    timeline: project.document.timeline.clone(),
                    trusted: trust,
                    owners,
                };
                let input_setup = input_bus.as_ref().map(|bus| inputs::Setup {
                    bus: bus.clone(),
                    routes: std::collections::BTreeMap::new(),
                });
                let armed = midi_recording::prepare_low_latency_with_inputs(
                    project,
                    &mut factory,
                    settings.clone(),
                    input_setup.as_ref(),
                )
                .map_err(|e| e.to_string())?;
                let output_bus = armed.controller.output_bus().cloned();
                let mut engine = Engine::default();
                engine.push(armed.processor);
                captured = Some((armed.controller, armed.recorder, connection, keyboard));
                Ok(OutputEngine {
                    output_bus,

                    input_bus: input_bus.clone(),
                    engine,
                    receivers,
                })
            })
            .map_err(|e| e.to_string())?;
        let (control, recorder, connection, keyboard) =
            captured.ok_or("MIDI engine did not prepare")?;
        self.control = Some(control);
        self.midi_recorder = Some(recorder);
        self.midi_connection = connection;
        self.midi_keyboard = Some(keyboard);
        if self.input_enabled {
            self.manager.set_monitor(true).map_err(|e| e.to_string())?;
        }
        self.midi_status = "Recording MIDI input into a new linear take".into();
        Ok(())
    }
    fn finish_midi_take(&mut self) -> Result<()> {
        if let Some(recorder) = &mut self.midi_recorder {
            let dropped = self
                .manager
                .telemetry()
                .dropped_events
                .load(std::sync::atomic::Ordering::Acquire);
            let take = recorder.finish(dropped)?;
            self.midi_recorder = None;
            self.midi_last = Some(take.clone());
            take.save_new(Path::new(&self.midi_folder))?;
            self.project
                .midi_takes
                .push(PathBuf::from(&self.midi_folder));
            let mut warning = String::new();
            match take.smf(960) {
                Ok(data) => {
                    std::fs::write(Path::new(&self.midi_folder).join("performance.mid"), data)?;
                }
                Err(e) => warning = format!("; SMF unavailable: {e}"),
            }
            let mut factory = Factory {
                buses: Default::default(),
                mode: soniclaw_host::studio::Mode::Live,
                spec: self.project.document.spec(),
                assets: self.project.assets.clone(),
                states: self.project.native_states.clone(),
                timeline: self.project.document.timeline.clone(),
                trusted: &self.trusted,
                owners: &mut self.owners,
            };
            match take.apply_prepared(&mut self.project, &mut factory) {
                Ok(prepared) => {
                    drop(prepared);
                    self.midi_status = format!(
                        "Saved MIDI take at {}; appended editable notes and controller points",
                        self.midi_folder
                    );
                }
                Err(e) => {
                    self.midi_status = format!(
                        "Raw MIDI saved at {}; clip not applied: {e}",
                        self.midi_folder
                    )
                }
            }
            self.midi_status.push_str(&warning);
            self.owners.clear();
            self.editor.clear_history();
        }
        Ok(())
    }
    fn begin_recording(&mut self) -> Result<()> {
        if self.recording {
            return Err("already recording".into());
        }
        self.stop()?;
        self.record_settings.validate(&self.project.document)?;
        if !self.input_enabled
            && self
                .record_settings
                .arms
                .iter()
                .any(|a| a.source == recording::Source::DryInput || a.monitor_input)
        {
            return Err("enable audio input before capturing or monitoring input".into());
        }
        self.configure_input()?;
        self.serial += 1;
        let path = PathBuf::from(format!("{}-take-{}", self.folder, self.serial));
        let id = format!("take-{}", self.serial);
        let mut captured = None;
        let port = self.midi_port;
        let mut midi = None;
        let project = &self.project;
        let settings = self.record_settings.clone();
        let input_bus = self.input_bus.clone();
        let trust = &self.trusted;
        let owners = &mut self.owners;
        self.manager
            .start(self.started.elapsed(), &mut |plan, telemetry| {
                if plan.spec().sample_rate != project.document.sample_rate as f64
                    || plan.spec().channels != 2
                    || plan.spec().max_block_frames != project.document.max_block_frames
                {
                    return Err("device must match project specification".into());
                }
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Live,
                    spec: plan.spec(),
                    assets: project.assets.clone(),
                    states: project.native_states.clone(),
                    timeline: project.document.timeline.clone(),
                    trusted: trust,
                    owners,
                };
                let (connection, keyboard, receivers) =
                    midi_inputs(port, telemetry).map_err(|e| e.to_string())?;
                midi = Some((connection, keyboard));
                let armed = recording::prepare_low_latency_with_inputs(
                    project,
                    &mut factory,
                    settings.clone(),
                    &path,
                    &id,
                    input_bus.clone(),
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(
                    path.join("recording-schedule.json"),
                    serde_json::to_vec_pretty(&armed.schedule)?,
                )?;
                let output_bus = armed.controller.output_bus().cloned();
                let mut engine = Engine::default();
                engine.push(armed.processor);
                captured = Some((armed.controller, armed.recorder, armed.progress));
                Ok(OutputEngine {
                    output_bus,

                    input_bus: input_bus.clone(),
                    engine,
                    receivers,
                })
            })
            .map_err(|e| e.to_string())?;
        let (control, recorder, progress) = captured.ok_or("recording device did not prepare")?;
        if let Some((connection, keyboard)) = midi {
            self.midi_connection = connection;
            self.midi_keyboard = Some(keyboard);
        }
        self.control = Some(control);
        self.record_progress = Some(progress);
        if self.input_enabled {
            self.manager.set_monitor(true).map_err(|e| e.to_string())?;
        }
        self.record_path = Some(path);
        self.recorder = Some(recorder);
        self.recording = true;
        Ok(())
    }
    fn finish_recording(&mut self) -> Result<()> {
        if self.recording {
            let mut take = self
                .recorder
                .as_mut()
                .ok_or("recorder unavailable")?
                .stop()?;
            self.recording = false;
            if let Some(path) = self.record_path.take() {
                let gaps = self
                    .record_progress
                    .as_ref()
                    .map_or(0, |p| p.status().input_discontinuities);
                take.input_discontinuities = gaps;
                std::fs::write(path.join("take.json"), serde_json::to_vec_pretty(&take)?)?;
                std::fs::write(
                    path.join("input-quality.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"schema":1,"input_discontinuities":gaps}),
                    )?,
                )?;
                self.project.takes.push(path.clone());
                if gaps == 0 && self.project.document.timeline.loop_range.is_none() {
                    let schedule: recording::Schedule = serde_json::from_slice(&std::fs::read(
                        path.join("recording-schedule.json"),
                    )?)?;
                    recording::place_input_take(&mut self.project, &path, &schedule)?;
                }
            }
            if let Some(c) = &mut self.control {
                c.detach_multitrack()?;
            }
            self.status = format!(
                "Recorded {} frames; {} missing; {} input discontinuities",
                take.frames, take.missing_frames, take.input_discontinuities
            );
        }
        Ok(())
    }
    fn act(&mut self, action: &str) -> Result<()> {
        if matches!(action, "undo-project" | "redo-project") {
            if self.recording || self.automation_writer.is_some() || self.midi_recorder.is_some() {
                return Err("finish recording/automation write before project undo".into());
            }
            self.stop()?;
            if action == "undo-project" {
                self.history.undo(&mut self.project)?;
            } else {
                self.history.redo(&mut self.project)?;
            }
            self.editor.clear_history();
            self.comp = None;
            self.selected = self.selected.min(self.project.document.tracks.len() - 1);
            return Ok(());
        }
        let previous = self.project.clone();
        self.act_inner(action)?;
        if matches!(action, "open" | "restore-recovery" | "new-project") {
            self.history.clear()?;
        } else {
            self.record_project(previous, action)?;
        }
        Ok(())
    }
    fn record_project(&mut self, previous: Project, label: &str) -> Result<()> {
        if let Err(error) = self.history.record(previous, &mut self.project, label) {
            // Shared live parameters may already have changed. Stop without recapturing them over rollback.
            self.manager.stop();
            self.control = None;
            self.owners.clear();
            self.editor.clear_history();
            return Err(error);
        }
        Ok(())
    }
    fn act_inner(&mut self, action: &str) -> Result<()> {
        if self.save_job.is_some() && matches!(action, "new-project" | "open" | "restore-recovery")
        {
            return Err("wait for the current Save to finish".into());
        }
        if action.starts_with("cues-") {
            return self.cues_action(action);
        }
        if action.starts_with("mix-") {
            return self.mix_action(action);
        }
        if action.starts_with("elastic-") {
            return self.elastic_action(action);
        }
        if action.starts_with("tools-") {
            return self.tools_action(action);
        }
        match action {
            "new-project" => {
                let candidate = workspace::blank(
                    self.new_project_name.clone(),
                    self.new_project_rate,
                    self.new_project_block,
                )?;
                self.stop()?;
                self.saved_baseline = None;
                self.project = candidate;
                self.selected = 0;
                self.midi_clip.clear();
                self.new_project_open = false;
                self.recovery_folder.clear();
                self.folder = format!("SoniclawProject-{}", self.serial);
                self.editor.clear_history();
                self.comp = None;
                self.autosave = None;
                self.autosave_enabled = false;
                self.recovery_store = None;
                self.record_settings = recording::Settings::for_document(&self.project.document);
                self.midi_last = None;
                self.routing_draft = None;
                self.wave_draft = None;
                self.wave_thumbnail = None;
                self.wave_ready = None;
                self.job_epoch += 1;
                self.wave_epoch += 1;
            }
            "workspace-command" => {
                self.stop()?;
                let command = self
                    .workspace_command
                    .take()
                    .ok_or("no workspace command")?;
                workspace::apply(&mut self.project, command)?;
                self.selected = self.selected.min(self.project.document.tracks.len() - 1);
                self.editor.clear_history();
                self.record_settings
                    .arms
                    .retain(|a| self.project.document.tracks.iter().any(|t| t.id == a.track));
            }
            "measure-loopback" => {
                self.stop()?;
                let reference = PathBuf::from(&self.latency_reference);
                let returned = PathBuf::from(&self.latency_return);
                let rate = self.project.document.sample_rate;
                self.latency_job = Some(self.jobs.submit(
                    "latency-measurement",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    8 * 1024 * 1024,
                    move |_| {
                        timing::measure_wav(&reference, &returned, rate).map_err(|e| e.to_string())
                    },
                )?);
            }
            "timeline-command" => {
                self.stop()?;
                let command = self.timing_command.take().ok_or("no timeline command")?;
                arrangement::apply(
                    &mut self.project,
                    command,
                    if self.audio_anchor_beats {
                        arrangement::AudioAnchor::Beats
                    } else {
                        arrangement::AudioAnchor::Seconds
                    },
                )?;
            }
            "set-timeline" => {
                self.stop()?;
                let timeline = self.timing_pending.take().ok_or("no timeline candidate")?;
                arrangement::change_timeline(
                    &mut self.project,
                    timeline,
                    if self.audio_anchor_beats {
                        arrangement::AudioAnchor::Beats
                    } else {
                        arrangement::AudioAnchor::Seconds
                    },
                )?;
            }
            "marker-upsert" => {
                self.stop()?;
                let id = if self.label_id.is_empty() {
                    workspace::identity("marker")?
                } else {
                    self.label_id.clone()
                };
                arrangement::apply(
                    &mut self.project,
                    arrangement::Command::Marker(arrangement::Marker {
                        id: id.clone(),
                        name: self.label_name.clone(),
                        beat: self.label_start,
                    }),
                    arrangement::AudioAnchor::Seconds,
                )?;
                self.label_id = id;
            }
            "section-upsert" => {
                self.stop()?;
                let id = if self.label_id.is_empty() {
                    workspace::identity("section")?
                } else {
                    self.label_id.clone()
                };
                arrangement::apply(
                    &mut self.project,
                    arrangement::Command::Section(arrangement::Section {
                        id: id.clone(),
                        name: self.label_name.clone(),
                        start: self.label_start,
                        end: self.label_end,
                    }),
                    arrangement::AudioAnchor::Seconds,
                )?;
                self.label_id = id;
            }
            "duplicate-arrangement" => {
                self.stop()?;
                let ids = arrangement::duplicate_selection(
                    &mut self.project,
                    &self.editor.selection.clips,
                    self.duplicate_beat,
                    self.duplicate_automation,
                )?;
                self.editor.clear_history();
                self.editor.selection.clips = ids;
            }
            "duplicate-section" => {
                self.stop()?;
                let ids = arrangement::duplicate_section(
                    &mut self.project,
                    &self.label_id,
                    self.duplicate_beat,
                    self.duplicate_automation,
                )?;
                self.editor.clear_history();
                self.editor.selection.clips = ids;
            }
            "locate" => {
                self.stop()?;
                let frame =
                    soniclaw::transport::TempoMap::compile(&self.project.document.timeline)?
                        .sample_at(
                            self.play_from_beat,
                            self.project.document.sample_rate as f64,
                        )?;
                self.editor.view.arrangement_scroll[0] =
                    (frame as f64 / self.editor.view.frames_per_pixel) as f32;
                self.status = format!("Located beat {:.3}; Play starts here", self.play_from_beat);
            }
            "midi-control-upsert" => {
                self.stop()?;
                let value = self.control_value.clamp(0., 1.);
                let control = match self.control_kind {
                    0 => midi_edit::Control::Cc {
                        controller: self.control_key,
                        value: (value * 127.).round() as u8,
                    },
                    1 => midi_edit::Control::Bend {
                        value: (value * 16383.).round() as u16,
                    },
                    2 => midi_edit::Control::Pressure {
                        value: (value * 127.).round() as u8,
                    },
                    _ => midi_edit::Control::PolyPressure {
                        key: self.control_key,
                        value: (value * 127.).round() as u8,
                    },
                };
                midi_edit::upsert(
                    &mut self.project,
                    &self.midi_clip,
                    midi_edit::ControlEvent {
                        id: self.control_id,
                        beat: self.control_beat,
                        channel: self.control_channel,
                        control,
                    },
                )?;
            }
            "midi-control-remove" => {
                self.stop()?;
                midi_edit::remove(&mut self.project, &self.midi_clip, self.control_id)?;
            }
            "midi-load-take" => {
                self.stop()?;
                let path = self
                    .project
                    .midi_takes
                    .get(self.midi_take_index)
                    .ok_or("choose a saved raw MIDI take")?;
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(path.join("midi-take.json"))?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_string(&mut text)?;
                self.midi_last = Some(midi_recording::Take::from_json(&text)?);
                self.midi_pass_index = 0;
            }
            "midi-choose-pass" => {
                self.stop()?;
                let take = self
                    .midi_last
                    .as_ref()
                    .ok_or("load or record a loop MIDI take first")?;
                let mut candidate = self.project.clone();
                take.apply_pass(&mut candidate, self.midi_pass_index)?;
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Live,
                    spec: candidate.document.spec(),
                    assets: candidate.assets.clone(),
                    states: candidate.native_states.clone(),
                    timeline: candidate.document.timeline.clone(),
                    trusted: &self.trusted,
                    owners: &mut self.owners,
                };
                drop(candidate.document.compile_streamed(
                    &candidate.assets,
                    &candidate.disk,
                    &mut factory,
                )?);
                self.owners.clear();
                self.project = candidate;
                self.editor.clear_history();
            }
            "group-comp" => {
                self.stop()?;
                let comp = self
                    .comp
                    .as_ref()
                    .ok_or("load a loop take into the comp editor")?;
                takes::apply_group(
                    &mut self.project,
                    &format!("take-group-{}", comp.take),
                    comp.take,
                    &comp.recipe,
                )?;
                self.editor.clear_history();
            }
            "ungroup-comp" => {
                self.stop()?;
                let id = self.ungroup_id.take().ok_or("select a comp group")?;
                takes::ungroup(&mut self.project, &id)?;
            }
            "toggle-comp-group" => {
                self.stop()?;
                let (id, on) = self.group_toggle.take().ok_or("select a comp group")?;
                takes::set_active(&mut self.project, &id, on)?;
            }
            "salvage-audio" => {
                self.stop()?;
                let source = PathBuf::from(&self.salvage_source);
                let target = PathBuf::from(&self.salvage_target);
                self.salvage_job = Some(self.jobs.submit(
                    "salvage-audio",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    2 * 1024 * 1024,
                    move |_| {
                        let report = soniclaw::record::salvage::recover(&source, &target)
                            .map_err(|e| e.to_string())?;
                        Ok((report, target))
                    },
                )?);
            }
            "midi-ports" => {
                self.midi_ports =
                    soniclaw::midi::native::input_ports().map_err(|e| e.to_string())?;
            }
            "midi-save-last" => {
                let take = self.midi_last.as_ref().ok_or("no MIDI take available")?;
                take.save_new(Path::new(&self.midi_folder))?;
                self.project
                    .midi_takes
                    .push(PathBuf::from(&self.midi_folder));
                if let Ok(data) = take.smf(960) {
                    std::fs::write(Path::new(&self.midi_folder).join("performance.mid"), data)?;
                }
                self.midi_status = format!("Saved last MIDI performance to {}", self.midi_folder);
            }
            "midi-record" => self.begin_midi_take()?,
            "midi-finish" => self.stop()?,
            "audit-media" => {
                self.stop()?;
                let project = self.project.clone();
                self.media_plan = None;
                self.media_audit_job = Some(self.jobs.submit(
                    "media-audit",
                    self.job_epoch,
                    jobs::Priority::Maintenance,
                    64 * 1024 * 1024,
                    move |c| {
                        if c.cancelled() {
                            return Err("audit cancelled".into());
                        }
                        media::audit(&project).map_err(|e| e.to_string())
                    },
                )?);
                self.media_status = "Auditing registered resources and dependencies".into();
            }
            "consolidate-media" => {
                self.stop()?;
                if self.media_copy_job.is_some() {
                    return Err("a consolidated copy is already running".into());
                }
                let plan = self.media_plan.clone().ok_or("run an audit first")?;
                if self.media_folder.is_empty() {
                    return Err("enter a new consolidated project folder".into());
                }
                let directory = PathBuf::from(&self.media_folder);
                let project = self.project.clone();
                self.media_copy_job = Some(self.jobs.submit(
                    "media-copy",
                    self.job_epoch,
                    jobs::Priority::Maintenance,
                    64 * 1024 * 1024,
                    move |c| {
                        if c.cancelled() {
                            return Err("copy cancelled".into());
                        }
                        media::consolidate(&project, &plan, &directory).map_err(|e| e.to_string())
                    },
                )?);
                self.media_status = "Copying portable project; originals remain unchanged".into();
            }
            "write-automation" => {
                if self.recording
                    || self.automation_writer.is_some()
                    || self.midi_recorder.is_some()
                {
                    return Err("finish the active take before arming automation".into());
                }
                let value = self
                    .control
                    .as_ref()
                    .and_then(|c| {
                        c.parameters()
                            .iter()
                            .find(|p| p.info().id == self.automation_parameter)
                    })
                    .ok_or("Play and select a live parameter first")?
                    .normalized();
                self.stop()?;
                let original = self.project.clone();
                automation_write::arm(
                    &mut self.project,
                    &self.automation_parameter,
                    value,
                    self.automation_mode,
                )?;
                let writer = automation_write::Writer::new(
                    &self.project,
                    &self.automation_parameter,
                    self.automation_grid,
                )?;
                if let Err(error) = self.play() {
                    self.manager.stop();
                    self.control = None;
                    self.owners.clear();
                    self.project = original;
                    return Err(error);
                }
                self.automation_writer = Some(writer);
            }
            "finish-automation" => self.stop()?,
            "discard-automation" => {
                if let Some(writer) = self.automation_writer.take() {
                    if let Some(c) = &self.control {
                        if let Some(g) = c.gestures().get(writer.lane_index()) {
                            g.clear();
                        }
                    }
                }
                self.stop()?;
            }
            "load-routing" => {
                self.stop()?;
                self.routing_draft = Some(routing::Draft::new(&self.project)?);
            }
            "commit-routing" => {
                self.stop()?;
                let draft = self.routing_draft.clone().ok_or("load routing first")?;
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Live,
                    spec: self.project.document.spec(),
                    assets: self.project.assets.clone(),
                    states: self.project.native_states.clone(),
                    timeline: self.project.document.timeline.clone(),
                    trusted: &self.trusted,
                    owners: &mut self.owners,
                };
                let prepared = draft.commit(&mut self.project, &mut factory);
                match prepared {
                    Ok(p) => {
                        drop(p);
                        self.owners.clear();
                        self.routing_draft = None;
                        self.status = "Routing prepared and applied; Play publishes it".into();
                    }
                    Err(e) => {
                        self.owners.clear();
                        return Err(e);
                    }
                }
            }
            "export" => {
                self.stop()?;
                if self.export_job.is_some() {
                    return Err("an export is already active".into());
                }
                let project = self.project.clone();
                let settings = self.export_settings.clone();
                let trusted = self.trusted.iter().cloned().collect::<Vec<_>>();
                settings.validate(&project)?;
                if self.export_folder.is_empty() {
                    return Err("enter a new export directory".into());
                }
                let directory = PathBuf::from(&self.export_folder);
                if directory.exists() {
                    return Err("export directory must be new".into());
                }
                self.export_job = Some(self.jobs.submit(
                    "export",
                    self.job_epoch,
                    jobs::Priority::Normal,
                    64 * 1024 * 1024,
                    move |ctx| {
                        // SAFETY: paths were explicitly approved by the user; the capability is constructed on this export worker.
                        let loader =
                            unsafe { soniclaw_host::formats::TrustedLoader::new(&trusted) }
                                .map_err(|e| e.to_string())?;
                        // SAFETY: the stopped session's explicit helper approvals are copied to this worker.
                        let helpers = unsafe {
                            soniclaw_host::studio::isolation_adapter::TrustedHelpers::new(&trusted)
                        }
                        .map_err(|e| e.to_string())?;
                        soniclaw_host::studio::render_export_with_helpers(
                            &project, settings, &directory, &loader, &helpers, &ctx,
                        )
                        .map_err(|e| e.to_string())
                    },
                )?);
                self.export_report = "Export queued; uses the stopped document snapshot".into();
            }
            "import-wave" => {
                self.stop()?;
                let path = PathBuf::from(&self.wave_import).canonicalize()?;
                // The bounded waveform decoder and warp builder validate actual samples later.
                self.serial += 1;
                let id = format!("source-{}", self.serial);
                let mut candidate = self.project.clone();
                candidate.disk.insert(id.clone(), path);
                candidate.validate()?;
                self.project = candidate;
                self.wave_source = id;
            }
            "load-wave" => {
                let source = self.wave_source.clone();
                let track = &self.project.document.tracks[self.selected].id;
                self.serial += 1;
                let asset = format!("warp-{}", self.serial);
                let region = format!("region-{}", self.serial);
                self.wave_draft = Some(wave_edit::Draft::new(
                    &self.project,
                    &source,
                    track,
                    &region,
                    &asset,
                )?);
                self.wave_thumbnail = None;
                self.wave_ready = None;
                self.wave_epoch += 1;
                let source = self.project.disk[&source].clone();
                let cache = PathBuf::from(format!("{}-waveforms", self.folder));
                self.wave_loader = Some(self.jobs.submit(
                    "waveform",
                    self.wave_epoch,
                    jobs::Priority::Interactive,
                    72 * 1024 * 1024,
                    move |ctx| {
                        soniclaw::waveform::load_or_build(
                            &source,
                            &cache,
                            Default::default(),
                            || ctx.cancelled(),
                            |p| ctx.progress(p.done, p.total),
                        )
                        .map_err(|e| e.to_string())
                    },
                )?);
            }
            "render-wave" => {
                if self.wave_job.is_some() {
                    return Err("finish/cancel the current warp job first".into());
                }
                self.serial += 1;
                let directory = PathBuf::from(format!("{}-warp-{}", self.folder, self.serial));
                let draft = self.wave_draft.clone().ok_or("load a source first")?;
                let project = self.project.clone();
                draft.validate(&project)?;
                self.wave_job = Some(self.jobs.submit(
                    "warp",
                    self.wave_epoch,
                    jobs::Priority::Normal,
                    64 * 1024 * 1024,
                    move |ctx| {
                        draft
                            .build(&project, &directory, &ctx.cancel_flag())
                            .map_err(|e| e.to_string())
                    },
                )?);
            }
            "apply-wave" => {
                self.stop()?;
                self.wave_ready
                    .take()
                    .ok_or("render a valid warp preview first")?
                    .apply(&mut self.project)?;
                self.editor.clear_history();
            }
            "enable-autosave" => {
                self.ensure_recovery()?;
                self.autosave_enabled = true;
                self.status="Autosave enabled; native owners must be stopped to capture current native state".into();
            }
            "disable-autosave" => {
                self.autosave_enabled = false;
                self.status = "Autosave disabled; any accepted snapshot is finishing".into();
            }
            "checkpoint" => {
                self.stop()?;
                self.ensure_recovery()?;
                let queued = self
                    .autosave
                    .as_mut()
                    .unwrap()
                    .checkpoint_now(self.started.elapsed(), &self.project)?;
                self.status = if queued {
                    "Checkpoint running".into()
                } else {
                    "A checkpoint is already running".into()
                };
            }
            "restore-recovery" => {
                self.ensure_recovery()?;
                if self.autosave.as_ref().is_some_and(|a| a.busy()) {
                    return Err("wait for the active checkpoint before restoring".into());
                }
                self.serial += 1;
                let path = PathBuf::from(format!("{}-recovered-{}", self.folder, self.serial));
                let restored = self
                    .recovery_store
                    .as_ref()
                    .unwrap()
                    .restore_latest(&path)?;
                self.stop()?;
                self.record_settings =
                    recording::Settings::for_document(&restored.project.document);
                self.media_plan = None;
                self.job_epoch += 1;
                self.wave_epoch += 1;
                self.wave_loader = None;
                self.wave_job = None;
                self.wave_draft = None;
                self.wave_thumbnail = None;
                self.wave_ready = None;
                self.project = restored.project;
                self.plugin_windows = Default::default();
                self.selected = 0;
                self.comp = None;
                self.editor.clear_history();
                self.status = format!(
                    "Restored generation {} into {}; {} recovery warnings",
                    restored.snapshot.generation,
                    path.display(),
                    restored.warnings.len()
                );
            }
            "inspect-media" => {
                self.resource_issues = Project::resource_issues(Path::new(&self.folder))?;
                self.repair_selected = 0;
                self.status = format!(
                    "{} missing or changed resource(s)",
                    self.resource_issues.len()
                );
            }
            "repair-media" => {
                let issue = self
                    .resource_issues
                    .get(self.repair_selected)
                    .ok_or("select a resource first")?
                    .clone();
                Project::repair_missing(
                    Path::new(&self.folder),
                    &issue.id,
                    issue.kind,
                    Path::new(&self.repair_path),
                )?;
                self.resource_issues = Project::resource_issues(Path::new(&self.folder))?;
                self.status="Resource restored with matching checksum; Open folder to load the verified project".into();
            }
            "play" => self.play()?,
            "stop" => self.stop()?,
            "record" => self.begin_recording()?,
            "end-take" => self.stop()?,
            "save" | "save-as" => {
                self.stop()?;
                self.project.layout.inputs = self
                    .record_settings
                    .arms
                    .iter()
                    .filter(|a| !a.channels.is_empty())
                    .map(|a| (a.track.clone(), a.channels.clone()))
                    .collect();
                if self.save_job.is_some() {
                    return Err("a save is already in progress".into());
                }
                let folder = PathBuf::from(&self.folder);
                if action == "save-as" && folder.exists() {
                    return Err("Save as needs a new destination folder".into());
                }
                let baseline = self.project.clone();
                let windows = self.plugin_windows.to_json().map_err(|e| e.to_string())?;
                let view = self.editor.view.to_json()?;
                self.save_job = Some(self.jobs.submit(
                    "session-save",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    256 * 1024 * 1024,
                    move |_ctx| {
                        let saved = soniclaw::studio::sessions::save(&baseline, &folder)
                            .map_err(|e| e.to_string())?;
                        std::fs::write(folder.join("plugin-windows.json"), windows)
                            .map_err(|e| e.to_string())?;
                        std::fs::write(folder.join("editor-view.json"), view)
                            .map_err(|e| e.to_string())?;
                        Ok(SaveResult {
                            baseline,
                            generation: saved.generation,
                        })
                    },
                )?);
                self.status = "Saving a portable session generation in the background".into();
            }
            "recover-pending-midi" => {
                self.stop()?;
                let (c, warnings) = soniclaw::studio::pending::Checkpoint::load(
                    Path::new(&format!("{}-pending", self.folder)),
                    &self.project.project_id,
                )?;
                let mut take = c
                    .midi
                    .ok_or("no pending MIDI performance")?
                    .accept_checkpoint()?;
                take.settings.clip = self.fresh_id("recovered-midi");
                self.serial += 1;
                let path = PathBuf::from(format!("{}-recovered-midi-{}", self.folder, self.serial));
                take.save_new(&path)?;
                let mut candidate = self.project.clone();
                take.apply(&mut candidate)?;
                candidate.midi_takes.push(path);
                candidate.validate()?;
                self.project = candidate;
                self.status = format!(
                    "Recovered lossless MIDI prefix; {} journal warnings",
                    warnings.len()
                );
            }
            "recover-pending-automation" => {
                self.stop()?;
                let (c, warnings) = soniclaw::studio::pending::Checkpoint::load(
                    Path::new(&format!("{}-pending", self.folder)),
                    &self.project.project_id,
                )?;
                let mut writer = automation_write::Writer::recover(
                    &self.project,
                    c.automation.ok_or("no pending automation performance")?,
                )?;
                writer.commit(&mut self.project)?;
                self.status = format!(
                    "Recovered automation take; {} journal warnings",
                    warnings.len()
                );
            }
            "open" => {
                let candidate =
                    Project::open(Path::new(&self.folder)).map_err(|e| e.to_string())?;
                let windows = load_plugin_windows(Path::new(&self.folder))?;
                let view_path = Path::new(&self.folder).join("editor-view.json");
                let view = if view_path.exists() {
                    use std::io::Read;
                    let mut bytes = String::new();
                    std::fs::File::open(view_path)?
                        .take(8193)
                        .read_to_string(&mut bytes)?;
                    ui::timeline::View::from_json(&bytes)?
                } else {
                    Default::default()
                };
                self.stop()?;
                self.autosave_enabled = false;
                self.autosave = None;
                self.recovery_store = None;
                self.record_settings = recording::Settings::for_document(&candidate.document);
                for arm in &mut self.record_settings.arms {
                    if let Some(ch) = candidate.layout.inputs.get(&arm.track) {
                        arm.channels = ch.clone();
                        arm.source = recording::Source::DryInput;
                    }
                }
                self.media_plan = None;
                self.job_epoch += 1;
                self.wave_epoch += 1;
                self.wave_loader = None;
                self.wave_job = None;
                self.wave_draft = None;
                self.wave_thumbnail = None;
                self.wave_ready = None;
                self.saved_baseline = Some(candidate.clone());
                self.project = candidate;
                self.plugin_windows = windows;
                self.selected = 0;
                self.editor.clear_history();
                self.editor.view = view;
                self.comp = None;
            }
            "freeze" => {
                self.stop()?;
                let id = self.project.document.tracks[self.selected].id.clone();
                self.project.document.tracks[self.selected].midi_input = false;
                self.serial += 1;
                let path = PathBuf::from(format!("{}-freeze-{}", self.folder, self.serial));
                if self.freeze_job.is_some() {
                    return Err("a freeze job is already running".into());
                }
                let native = self
                    .project
                    .document
                    .tracks
                    .iter()
                    .flat_map(|t| t.instrument.iter().chain(&t.inserts))
                    .chain(
                        self.project
                            .document
                            .auxiliaries
                            .iter()
                            .flat_map(|a| &a.inserts),
                    )
                    .chain(self.project.document.mix_routing.descriptors())
                    .any(|d| d.kind == "clap");
                if !native {
                    let project = self.project.clone();
                    let monitor = freeze::Monitor::default();
                    let m = monitor.clone();
                    let ticket = self.jobs.submit(
                        "freeze",
                        self.job_epoch,
                        jobs::Priority::Normal,
                        64 * 1024 * 1024,
                        move |ctx| {
                            freeze::render_monitored(
                                &project.document,
                                &id,
                                &project.assets,
                                &project.disk,
                                &mut stock::AssetsFactory {
                                    assets: &project.assets,
                                },
                                &path,
                                0,
                                project.document.sample_rate as u64 * 4,
                                project.document.sample_rate as u64,
                                &m.with_cancel(ctx.cancel_flag()),
                            )
                            .map_err(|e| e.to_string())
                        },
                    )?;
                    self.freeze_job = Some((ticket, monitor));
                    self.status = "Freeze running; edits invalidate stale results".into();
                    return Ok(());
                }
                let mut factory = Factory {
                    buses: Default::default(),
                    mode: soniclaw_host::studio::Mode::Offline,
                    spec: self.project.document.spec(),
                    assets: self.project.assets.clone(),
                    states: self.project.native_states.clone(),
                    timeline: self.project.document.timeline.clone(),
                    trusted: &self.trusted,
                    owners: &mut self.owners,
                };
                self.project
                    .freeze(
                        &id,
                        &mut factory,
                        &path,
                        0,
                        self.project.document.sample_rate as u64 * 4,
                        self.project.document.sample_rate as u64,
                    )
                    .map_err(|e| e.to_string())?;
                self.owners.clear();
            }
            "unfreeze" => {
                self.stop()?;
                let id = self.project.document.tracks[self.selected].id.clone();
                self.project.unfreeze(&id).map_err(|e| e.to_string())?;
            }
            "apply" => {
                let running = self.manager.is_running();
                self.stop()?;
                self.project.document.validate()?;
                if running {
                    self.play()?;
                }
            }
            "apply-comp" => {
                self.stop()?;
                let draft = self.comp.as_ref().ok_or("load loop passes first")?;
                self.project
                    .apply_comp(
                        &self.comp_target,
                        soniclaw::studio::project::Comp {
                            take: draft.take,
                            stem: draft.stem.clone(),
                            recipe: draft.recipe.clone(),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                self.editor.clear_history();
            }
            "add-plugin" => {
                self.stop()?;
                let path = PathBuf::from(&self.plugin_path).canonicalize()?;
                if !self.trusted.contains(&path) {
                    return Err("restart with the module path explicitly allowlisted".into());
                }
                self.serial += 1;
                let descriptor = Descriptor {
                    id: format!("native.{}", self.serial),
                    kind: "clap".into(),
                    state: serde_json::json!({"path":path,"plugin_id":self.plugin_id}),
                };
                let mut candidate = self.project.document.clone();
                candidate.tracks[self.selected].inserts.push(descriptor);
                candidate.validate()?;
                self.project.document = candidate;
            }
            _ => {}
        }
        Ok(())
    }
}
impl Studio {
    fn ensure_recovery(&mut self) -> Result<()> {
        if self.recovery_folder.is_empty() {
            self.recovery_folder = format!("{}-recovery", self.folder);
        }
        if self.recovery_store.is_none() {
            self.recovery_store = Some(recovery::Store::open(
                Path::new(&self.recovery_folder),
                &self.project.project_id,
                recovery::Limits::default(),
            )?);
        }
        if self.autosave.is_none() {
            self.autosave = Some(recovery::Autosave::new(
                self.recovery_store.as_ref().unwrap().clone(),
                Duration::from_secs(self.autosave_seconds),
            )?);
        }
        Ok(())
    }
    fn poll_session(&mut self) {
        if let Some(done) = self.save_job.as_mut().and_then(|t| t.poll(self.job_epoch)) {
            self.save_job = None;
            match done {
                Ok(s) => {
                    self.saved_baseline = Some(s.baseline);
                    self.status = format!("Saved session generation {}", s.generation);
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(done) = self
            .pending_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.pending_job = None;
            if let Err(e) = done {
                self.status = format!("Pending journal: {e}");
            }
        }
        if self.pending_job.is_none()
            && self.pending_at.elapsed() >= Duration::from_secs(2)
            && (self.midi_recorder.is_some() || self.automation_writer.is_some())
        {
            self.pending_at = Instant::now();
            let dropped = self
                .manager
                .telemetry()
                .dropped_events
                .load(std::sync::atomic::Ordering::Acquire);
            let midi = match self
                .midi_recorder
                .as_mut()
                .map(|r| r.checkpoint(dropped))
                .transpose()
            {
                Ok(t) => t,
                Err(e) => {
                    self.status = e.to_string();
                    return;
                }
            };
            let checkpoint = soniclaw::studio::pending::Checkpoint {
                schema: 1,
                project_id: self.project.project_id.clone(),
                midi,
                automation: self.automation_writer.as_ref().map(|w| w.checkpoint()),
            };
            let path = PathBuf::from(format!("{}-pending", self.folder));
            match self.jobs.submit(
                "pending-performance",
                self.job_epoch,
                jobs::Priority::Maintenance,
                64 * 1024 * 1024,
                move |_| checkpoint.save(&path).map_err(|e| e.to_string()),
            ) {
                Ok(t) => self.pending_job = Some(t),
                Err(e) => self.status = e.to_string(),
            }
        }
    }
    fn poll_recovery(&mut self) {
        if let Some(result) = self.autosave.as_mut().and_then(recovery::Autosave::poll) {
            self.status = match result {
                Ok(s) => format!(
                    "Saved recovery generation {}; {} retention warnings",
                    s.snapshot.generation,
                    s.warnings.len()
                ),
                Err(e) => format!("Recovery checkpoint failed: {e}"),
            };
        }
        if self.autosave_enabled && self.owners.is_empty() {
            if let Some(a) = &mut self.autosave {
                if let Err(e) = a.tick(self.started.elapsed(), &self.project) {
                    self.status = e.to_string();
                }
            }
        }
    }
    fn poll_freeze(&mut self) {
        if let Some(result) = self
            .freeze_job
            .as_mut()
            .and_then(|(ticket, _)| ticket.poll(self.job_epoch))
        {
            self.freeze_job = None;
            let previous = self.project.clone();
            match result
                .map_err(|e| e.to_string().into())
                .and_then(|f| self.project.apply_frozen(f))
                .and_then(|()| self.record_project(previous, "Freeze track"))
            {
                Ok(()) => {
                    self.editor.clear_history();
                    self.status = "Freeze completed and verified".into();
                }
                Err(e) => self.status = format!("Freeze was not applied: {e}"),
            }
        }
    }
    fn show(&mut self, root: &mut egui::Ui) {
        self.poll_tools();
        self.poll_elastic();
        if let Some(result) = self
            .latency_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.latency_job = None;
            match result {
                Ok(m) => {
                    let previous = self.project.clone();
                    let c = self.project.layout.calibration.get_or_insert_with(|| {
                        timing::Calibration::new(self.project.document.sample_rate)
                    });
                    c.audio_offset_frames = m.offset_frames;
                    c.label = "Electrical impulse loopback".into();
                    if let Err(e) = self.record_project(previous, "Measure loopback") {
                        self.status = e.to_string();
                    } else {
                        self.status=format!("Measured {} frames, peak/noise {:.1}; verify this interface and buffer configuration",m.offset_frames,m.peak_to_noise);
                    }
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(result) = self
            .salvage_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.salvage_job = None;
            match result {
                Ok((r, path)) => {
                    self.wave_import = path.to_string_lossy().into_owned();
                    self.status=format!("Recovered {} whole frames; {} trailing bytes discarded. Audition/import the new WAV from the audio editor.",r.recovered_frames,r.discarded_bytes);
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        self.poll_session();
        self.poll_freeze();
        self.sync_controls();
        self.poll_recovery();
        if let Some(result) = self
            .media_audit_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.media_audit_job = None;
            match result {
                Ok(plan) => {
                    self.media_status =
                        "Audit ready; changes require a fresh audit before copying".into();
                    self.media_plan = Some(plan);
                }
                Err(e) => self.media_status = e.to_string(),
            }
        }
        if let Some(result) = self
            .media_copy_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.media_copy_job = None;
            self.media_status=match result{Ok(copy)=>format!("Consolidated copy reopened and verified: {} assets retained, {} omitted. Originals untouched.",copy.retained_assets,copy.omitted_assets),Err(e)=>e.to_string()};
        }

        if let Some(result) = self
            .export_job
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.export_job = None;
            self.export_report = match result {
                Ok(r) => format!(
                    "Exported {} files at {} Hz; see export.json",
                    r.files.len(),
                    r.sample_rate
                ),
                Err(e) => format!("Export: {e}"),
            };
        }

        if let Some(done) = self
            .wave_loader
            .as_mut()
            .and_then(|t| t.poll(self.wave_epoch))
        {
            self.wave_loader = None;
            match done {
                Ok(loaded) => self.wave_thumbnail = Some(loaded.thumbnail),
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(done) = self.wave_job.as_mut().and_then(|t| t.poll(self.wave_epoch)) {
            self.wave_job = None;
            match done {
                Ok(ready) => {
                    self.wave_ready = Some(ready);
                    self.status = "Warp rendered; Apply cache to add/update the region".into();
                }
                Err(e) => self.status = e.to_string(),
            }
        }

        let ctx = root.ctx().clone();
        self.sync_controls();
        let previous = self.project.clone();
        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        if !pointer_down {
            for (key, channel) in std::mem::take(&mut self.midi_held) {
                if let Some(sender) = &mut self.midi_keyboard {
                    sender.send(MidiMessage::NoteOff {
                        channel,
                        note: key,
                        velocity: 0,
                    });
                }
            }
        }
        self.editor.show_history_controls = false;
        if let Some(c) = &mut self.control {
            c.collect_retired();
        }
        for native in &mut self.owners {
            if let Err(e) = native.poll() {
                self.status = e.to_string();
            }
            if let Ok(state) = self.plugin_windows.for_instance(&native.id, &native.entry) {
                if native.restore_window {
                    native.restore_window = false;
                    if state.reopen {
                        match native.open_editor(state) {
                            Ok(w) => {
                                if !w.is_empty() {
                                    self.status = w.join("; ");
                                }
                            }
                            Err(e) => {
                                state.reopen = false;
                                self.status = e.to_string();
                            }
                        }
                    }
                } else if state.reopen && !native.editor_open() {
                    state.reopen = false;
                }
            }
            for event in native.drain_editor_events() {
                match event {
                    soniclaw_host::editors::EditorEvent::Error { message, .. } => {
                        self.status = message
                    }
                    soniclaw_host::editors::EditorEvent::Closed { .. } => {
                        if let Ok(s) = self.plugin_windows.for_instance(&native.id, &native.entry) {
                            s.reopen = false;
                        }
                    }
                    soniclaw_host::editors::EditorEvent::Resized { size, .. } => {
                        if let Ok(s) = self.plugin_windows.for_instance(&native.id, &native.entry) {
                            s.size = Some(size);
                        }
                    }
                }
            }
        }
        if let Err(e) = self.service_native_edits(false) {
            self.status = e.to_string();
        }
        if self.recording
            && self
                .record_progress
                .as_ref()
                .is_some_and(|p| p.status().punch_complete)
        {
            if let Err(e) = self.stop() {
                self.status = e.to_string();
            }
        }
        let mut action = None;
        if let Some(recorder) = &mut self.midi_recorder {
            recorder.drain();
            if recorder.status().limit_reached {
                action = Some("midi-finish");
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            action = Some("save");
        }
        if !ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                action = Some(if self.control.is_some() {
                    "stop"
                } else {
                    "play"
                });
            }
            let redo = ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                ) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
            });
            let undo =
                !redo && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
            if redo {
                action = Some("redo-project");
            } else if undo {
                action = Some("undo-project");
            }
        }
        ui::apply_theme(&ctx);
        root.heading("Soniclaw · Reference Studio");
        root.horizontal(|ui| {
            let dirty = self
                .saved_baseline
                .as_ref()
                .is_none_or(|p| !history::equivalent(p, &self.project).unwrap_or(false));
            ui.label(format!(
                "{}{}",
                self.project.name,
                if dirty { " · unsaved" } else { "" }
            ));
            if ui.button("New project").clicked() {
                self.new_project_open = true;
            }
            if ui
                .add_enabled(self.save_job.is_none(), egui::Button::new("Save"))
                .clicked()
            {
                action = Some("save");
            }
            if ui
                .add_enabled(self.save_job.is_none(), egui::Button::new("Save as"))
                .clicked()
            {
                action = Some("save-as");
            }
            if ui.button("Open").clicked() {
                action = Some("open");
            }
        });
        if self.new_project_open {
            egui::Window::new("New project").open(&mut self.new_project_open).show(&ctx,|ui|{ui.label("Save the current project first if you want to keep unsaved work. Creating a project replaces the current session.");ui.text_edit_singleline(&mut self.new_project_name);ui.add(egui::DragValue::new(&mut self.new_project_rate).range(8000..=192000).suffix(" Hz"));ui.add(egui::DragValue::new(&mut self.new_project_block).range(1..=1024).suffix(" max block frames"));
if ui.button("Create and replace current session").clicked(){action=Some("new-project");}});
        }
        root.collapsing("Tracks and inspector", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add audio track").clicked() {
                    let id = self.fresh_id("audio");
                    self.workspace_command = Some(workspace::Command::Add {
                        id,
                        name: "Audio".into(),
                        kind: workspace::Kind::Audio,
                    });
                    action = Some("workspace-command");
                }
                if ui.button("Add instrument track").clicked() {
                    let id = self.fresh_id("instrument");
                    self.workspace_command = Some(workspace::Command::Add {
                        id,
                        name: "Instrument".into(),
                        kind: workspace::Kind::Instrument,
                    });
                    action = Some("workspace-command");
                }
            });
            let id = self.project.document.tracks[self.selected].id.clone();
            ui.horizontal(|ui| {
                ui.label(format!("Selected: {id}"));
                ui.text_edit_singleline(&mut self.track_name);
                if ui.button("Rename").clicked() {
                    self.workspace_command = Some(workspace::Command::Rename {
                        id: id.clone(),
                        name: self.track_name.clone(),
                    });
                    action = Some("workspace-command");
                }
                if ui.button("Duplicate").clicked() {
                    let new_id = self.fresh_id("track");
                    self.workspace_command = Some(workspace::Command::Duplicate {
                        id: id.clone(),
                        new_id,
                    });
                    action = Some("workspace-command");
                }
                if ui
                    .add_enabled(
                        self.project.document.tracks.len() > 1,
                        egui::Button::new("Delete selected track"),
                    )
                    .clicked()
                {
                    self.workspace_command = Some(workspace::Command::Delete { id: id.clone() });
                    action = Some("workspace-command");
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.selected > 0, egui::Button::new("Move up"))
                    .clicked()
                {
                    self.workspace_command = Some(workspace::Command::Move {
                        id: id.clone(),
                        index: self.selected - 1,
                    });
                    self.selected -= 1;
                    action = Some("workspace-command");
                }
                if ui
                    .add_enabled(
                        self.selected + 1 < self.project.document.tracks.len(),
                        egui::Button::new("Move down"),
                    )
                    .clicked()
                {
                    self.workspace_command = Some(workspace::Command::Move {
                        id: id.clone(),
                        index: self.selected + 1,
                    });
                    self.selected += 1;
                    action = Some("workspace-command");
                }
            });
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.track_folder_name);
                if ui.button("Create track folder").clicked() {
                    let id = self.fresh_id("folder");
                    self.workspace_command = Some(workspace::Command::Folder {
                        id,
                        name: self.track_folder_name.clone(),
                    });
                    action = Some("workspace-command");
                }
            });
            let mut folder = self.project.layout.membership.get(&id).cloned();
            let previous = folder.clone();
            egui::ComboBox::from_id_salt("track-folder")
                .selected_text(
                    folder
                        .as_ref()
                        .and_then(|f| self.project.layout.folders.get(f))
                        .map_or("No folder", String::as_str),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut folder, None, "No folder");
                    for (f, name) in &self.project.layout.folders {
                        ui.selectable_value(&mut folder, Some(f.clone()), name);
                    }
                });
            if folder != previous {
                self.workspace_command = Some(workspace::Command::AssignFolder {
                    track: id.clone(),
                    folder,
                });
                action = Some("workspace-command");
            }
            ui.checkbox(
                &mut self.project.document.tracks[self.selected].midi_input,
                "Monitor live MIDI on this track",
            );
            if ui
                .add_enabled(
                    self.project.document.tracks[self.selected]
                        .instrument
                        .is_some(),
                    egui::Button::new("Add four-beat MIDI clip"),
                )
                .clicked()
            {
                let clip = self.fresh_id("clip");
                self.midi_clip = clip.clone();
                self.workspace_command = Some(workspace::Command::AddMidi {
                    track: id,
                    id: clip,
                    start: 0.,
                    length: 4.,
                });
                action = Some("workspace-command");
            }
        });

        self.tools_ui(root, &mut action);
        self.elastic_ui(root, &mut action);
        self.mix_ui(root, &mut action);
        self.cues_ui(root, &mut action);
        self.native_ui(root);
        self.native_bus_ui(root);
        self.isolation_ui(root);
        let js = self.jobs.stats();
        root.label(format!(
            "Jobs: {} active, {} queued, {} retained · {} MiB reserved",
            js.active,
            js.queued,
            js.retained,
            js.reserved_bytes / 1024 / 1024
        ));
        root.label("Shared session engine · arrangement · mixer · takes · freeze");
        root.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.history.undo_label().is_some()
                        && !self.recording
                        && self.automation_writer.is_none()
                        && self.midi_recorder.is_none(),
                    egui::Button::new("Undo project edit"),
                )
                .clicked()
            {
                action = Some("undo-project");
            }
            if ui
                .add_enabled(
                    self.history.redo_label().is_some()
                        && !self.recording
                        && self.automation_writer.is_none()
                        && self.midi_recorder.is_none(),
                    egui::Button::new("Redo project edit"),
                )
                .clicked()
            {
                action = Some("redo-project");
            }
            if let Ok(s) = self.history.stats() {
                ui.label(format!(
                    "History: {} undo · {} redo · ~{} KiB retained",
                    s.undo,
                    s.redo,
                    s.estimated_retained_bytes / 1024
                ));
            }
        });
        if let Some(job) = &self.freeze_job {
            let (done, total) = job.1.progress();
            root.label(format!("Freeze progress: {done}/{total} frames"));
            if root.button("Cancel freeze").clicked() {
                job.0.cancel();
                job.1.cancel();
            }
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        root.horizontal(|ui| {
            for (id, title) in [
                ("play", "Play / restart"),
                ("stop", "Stop"),
                ("record", "Record stems"),
                ("end-take", "Finish take"),
                ("apply", "Apply edits / restart"),
            ] {
                if ui.button(title).clicked() {
                    action = Some(id);
                }
            }
        });
        root.collapsing("MIDI input and performance recording",|ui|{
            ui.add_enabled_ui(self.midi_recorder.is_none(),|ui|{
                if ui.button("Refresh MIDI input ports").clicked(){action=Some("midi-ports");}
                egui::ComboBox::from_id_salt("midi-port").selected_text(self.midi_port.and_then(|i|self.midi_ports.get(i)).map_or("On-screen keyboard only",String::as_str)).show_ui(ui,|ui|{ui.selectable_value(&mut self.midi_port,None,"On-screen keyboard only");for(i,name)in self.midi_ports.iter().enumerate(){ui.selectable_value(&mut self.midi_port,Some(i),name);}});
                egui::ComboBox::from_id_salt("midi-record-track").selected_text(&self.midi_settings.track).show_ui(ui,|ui|{for t in self.project.document.tracks.iter().filter(|t|t.instrument.is_some()){ui.selectable_value(&mut self.midi_settings.track,t.id.clone(),&t.id);}});
                ui.horizontal(|ui|{ui.label("Start beat");ui.add(egui::DragValue::new(&mut self.midi_settings.start_beat).range(0.0..=1_000_000.));ui.label("Onset grid (beats)");ui.add(egui::DragValue::new(&mut self.midi_settings.grid_beats).range(0.0..=16.).speed(0.0625));ui.checkbox(&mut self.midi_settings.quantize_ends,"Quantize ends");});
                let mut filter=self.midi_settings.channel.is_some();
if ui.checkbox(&mut filter,"Record only this MIDI channel").changed(){self.midi_settings.channel=if filter{Some(0)}else{None};}
                if let Some(channel)=&mut self.midi_settings.channel{ui.add(egui::DragValue::new(channel).range(0..=15).prefix("Zero-based channel "));}
                ui.add(egui::DragValue::new(&mut self.midi_settings.max_seconds).range(1..=14400).suffix(" second maximum"));ui.text_edit_singleline(&mut self.midi_folder);
                if ui.button("Start MIDI take (new folder)").clicked(){action=Some("midi-record");}
            });
            if let Some(recorder)=&self.midi_recorder{let s=recorder.status();ui.label(format!("{} input events · {} queued/drained · {} lost · {} unsupported",s.accepted_events,s.drained_events,s.lost_events,s.unsupported_messages));
if ui.button("Finish MIDI take").clicked(){action=Some("midi-finish");}}
            ui.horizontal(|ui|{for(key,name)in [(60,"C4"),(62,"D"),(64,"E"),(65,"F"),(67,"G"),(69,"A"),(71,"B"),(72,"C5")]{let response=ui.add_enabled(self.midi_keyboard.is_some(),egui::Button::new(name).selected(self.midi_held.contains_key(&key)));let down=response.is_pointer_button_down_on();
if down&&!self.midi_held.contains_key(&key){if let Some(sender)=&mut self.midi_keyboard{sender.send(MidiMessage::NoteOn{channel:self.midi_settings.channel.unwrap_or(0),note:key,velocity:100});self.midi_held.insert(key,self.midi_settings.channel.unwrap_or(0));}}else if !down{if let Some(channel)=self.midi_held.remove(&key){if let Some(sender)=&mut self.midi_keyboard{sender.send(MidiMessage::NoteOff{channel,note:key,velocity:0});}}}}});
            if ui.add_enabled(self.midi_last.is_some()&&self.midi_recorder.is_none(),egui::Button::new("Save last raw take to new folder")).clicked(){action=Some("midi-save-last");}
            ui.label("Hold a key while playing or recording. Input ports are fixed on Play/Record restart. Enable loop mode under Take groups for loop recording. Native MIDI uses backend timestamps with one internal block of scheduling lookahead.");ui.label(&self.midi_status);
        });
        root.collapsing("Media audit and consolidation",|ui|{
            if ui.button("Audit registered media").clicked(){action=Some("audit-media");}
            if let Some(plan)=&self.media_plan{let report=plan.report();ui.label(format!("{} resources · {} KiB unused decoded memory · {} KiB unused disk file bytes",report.entries.len(),report.removable_memory_bytes/1024,report.removable_disk_bytes/1024));for warning in &report.warnings{ui.label(warning);}
                egui::ScrollArea::vertical().id_salt("media-audit-rows").max_height(160.).show(ui,|ui|{for row in &report.entries{ui.label(format!("{:?} {} · {} bytes · {}",row.storage,row.id,row.bytes,if row.referenced{"retained"}else{"unused in copy"}));}});
            }
            ui.text_edit_singleline(&mut self.media_folder);
            if ui.add_enabled(self.media_plan.is_some()&&self.media_copy_job.is_none(),egui::Button::new("Write consolidated copy to new folder")).clicked(){action=Some("consolidate-media");}
            ui.label("Original files are never deleted. Disabled regions, freeze history and takes are retained. Opaque native/custom state keeps all registered assets. Copy/reopen can delay cancellation until filesystem work returns.");
            ui.label(&self.media_status);
        });
        root.collapsing("Automation write",|ui|{
            ui.add_enabled_ui(self.automation_writer.is_none(),|ui|{
                egui::ComboBox::from_id_salt("write-parameter").selected_text(&self.automation_parameter).show_ui(ui,|ui|{if let Some(c)=&self.control{for p in c.parameters(){ui.selectable_value(&mut self.automation_parameter,p.info().id.to_string(),p.info().id);}}});
                ui.horizontal(|ui|{ui.selectable_value(&mut self.automation_mode,soniclaw::automation::Mode::Touch,"Touch");ui.selectable_value(&mut self.automation_mode,soniclaw::automation::Mode::Latch,"Latch");ui.add(egui::DragValue::new(&mut self.automation_grid).range(0.0..=16.).speed(0.0625).suffix(" beat grid"));});
                if ui.button("Arm lane and restart writing").clicked(){action=Some("write-automation");}
            });
            if let Some(writer)=&self.automation_writer{ui.label(format!("Writing {} · {} pending points",writer.parameter(),writer.point_count()));
if ui.button("Finish automation and stop").clicked(){action=Some("finish-automation");}
if ui.button("Discard pending write").clicked(){action=Some("discard-automation");}}
            ui.label("Play first, select a live parameter, arm, then move its fader. GUI-clock sampling; committed on Stop. Other arrangement/automation edits invalidate a pending write.");
        });
        root.collapsing("Routing matrix",|ui|{
            if ui.button("Load routing from stopped project").clicked(){action=Some("load-routing");}
            if let Some(draft)=&mut self.routing_draft{
                let mut edit=None;
                ui.label("Tracks feed master directly and up to four parallel auxiliary returns. No feedback edges.");
                egui::Grid::new("routing-matrix").striped(true).show(ui,|ui|{
                    ui.label("Track → master + sends");for i in 0..draft.document.auxiliaries.len(){ui.label(format!("Aux {}",i+1));}ui.end_row();
                    for strip in &draft.document.mixer.strips{ui.label(&strip.name);for(i,g)in strip.sends.iter().enumerate(){let mut gain=*g;
if ui.add(egui::DragValue::new(&mut gain).range(0.0..=4.).speed(0.01)).changed(){edit=Some(routing::Edit::Send{track:strip.id.clone(),aux:i,gain});}}ui.end_row();}
                });
                for (i,aux) in draft.document.auxiliaries.iter().enumerate(){ui.horizontal(|ui|{
                    ui.label(format!("Aux {} → master",i+1));let mut pre=matches!(draft.document.mixer.taps[i],soniclaw::mixer::Tap::PreFader);
if ui.checkbox(&mut pre,"Pre-fader tap").changed(){edit=Some(routing::Edit::Tap{index:i,tap:if pre{soniclaw::mixer::Tap::PreFader}else{soniclaw::mixer::Tap::PostFader}});}
                    let mut gain=aux.return_gain;
if ui.add(egui::DragValue::new(&mut gain).range(0.0..=4.).speed(0.01)).changed(){edit=Some(routing::Edit::Return{aux:i,gain});}
                    if ui.button("Add gain insert (0.5)").clicked(){self.serial+=1;edit=Some(routing::Edit::Insert{aux:i,descriptor:Descriptor{id:format!("aux-fx-{}",self.serial),kind:"gain".into(),state:serde_json::json!({"value":0.5})}});}
                    if ui.button("Remove aux").clicked(){edit=Some(routing::Edit::RemoveAux{index:i});}
                });for(j,d)in aux.inserts.iter().enumerate(){ui.horizontal(|ui|{ui.label(format!("{}: {}",d.id,d.kind));
if ui.button("Remove insert").clicked(){edit=Some(routing::Edit::RemoveInsert{aux:i,index:j});}});}}
                if ui.add_enabled(draft.document.auxiliaries.len()<4,egui::Button::new("Add auxiliary")).clicked(){edit=Some(routing::Edit::AddAux{tap:soniclaw::mixer::Tap::PostFader,return_gain:1.});}
                if let Some(edit)=edit{if let Err(e)=draft.edit(edit){self.status=e.to_string();}}
                if ui.button("Prepare and apply routing").clicked(){action=Some("commit-routing");}
            }
        });
        root.collapsing("Export centre",|ui|{
            ui.add_enabled_ui(self.export_job.is_none(),|ui|{
                ui.text_edit_singleline(&mut self.export_folder);
                let mut convert=self.export_settings.output_rate.is_some();
if ui.checkbox(&mut convert,"Convert output sample rate").changed(){self.export_settings.output_rate=if convert{Some(44100)}else{None};}
if let Some(rate)=&mut self.export_settings.output_rate{ui.add(egui::DragValue::new(rate).range(8000..=192000).suffix(" Hz"));}
                ui.checkbox(&mut self.export_settings.mix,"Full mix");
                for t in &self.project.document.tracks{let mut on=self.export_settings.stems.contains(&t.id);
if ui.checkbox(&mut on,format!("Pre-mixer stem: {}",t.id)).changed(){if on{self.export_settings.stems.push(t.id.clone());}else{self.export_settings.stems.retain(|id|id!=&t.id);}}}
                ui.label("Same-pass routed stems (post-strip/bus/return, before master inserts):");
                let sources=self.project.document.tracks.iter().map(|t|delivery::Source::TrackPost(t.id.clone())).chain(self.project.document.mix_routing.buses.iter().map(|b|delivery::Source::Bus(b.id.clone()))).chain((0..self.project.document.auxiliaries.len()).map(delivery::Source::Aux)).collect::<Vec<_>>();
                for source in sources {let mut on=self.export_settings.routed_stems.contains(&source);if ui.checkbox(&mut on,format!("{source:?}")).changed(){if on{self.export_settings.routed_stems.push(source);}else{self.export_settings.routed_stems.retain(|s|s!=&source);}}}
                ui.horizontal(|ui|{ui.label("Start / length / tail (frames)");ui.add(egui::DragValue::new(&mut self.export_settings.start_frame));ui.add(egui::DragValue::new(&mut self.export_settings.length_frames));ui.add(egui::DragValue::new(&mut self.export_settings.tail_frames));});
                ui.horizontal(|ui|{for (f,label) in [(export::Format::Wav,"WAV"),(export::Format::Aiff,"AIFF"),(export::Format::Flac,"FLAC")]{ui.selectable_value(&mut self.export_settings.format,f,label);}for (d,label) in [(export::Depth::Float32,"Float32"),(export::Depth::Pcm16,"PCM16"),(export::Depth::Pcm24,"PCM24")]{ui.selectable_value(&mut self.export_settings.depth,d,label);}});
                let mut norm=match self.export_settings.normalize{export::Normalize::None=>0,export::Normalize::Peak(_)=>1,export::Normalize::Lufs(_)=>2};ui.horizontal(|ui|{ui.selectable_value(&mut norm,0,"No normalization");ui.selectable_value(&mut norm,1,"Peak");ui.selectable_value(&mut norm,2,"LUFS (48 kHz output)");});
                match norm{0=>self.export_settings.normalize=export::Normalize::None,1=>{if !matches!(self.export_settings.normalize,export::Normalize::Peak(_)){self.export_settings.normalize=export::Normalize::Peak(0.9);}
if let export::Normalize::Peak(v)=&mut self.export_settings.normalize{ui.add(egui::Slider::new(v,0.01..=1.).text("Peak target"));}},_=>{if !matches!(self.export_settings.normalize,export::Normalize::Lufs(_)){self.export_settings.normalize=export::Normalize::Lufs(-14.);}
if let export::Normalize::Lufs(v)=&mut self.export_settings.normalize{ui.add(egui::Slider::new(v,-60.0..=-5.).text("Target LUFS"));}}}
                ui.add(egui::Slider::new(&mut self.export_settings.peak_ceiling,0.01..=1.).text("Normalization sample-peak ceiling"));ui.checkbox(&mut self.export_settings.dither,"Deterministic TPDF dither for integer output");
                ui.label("Disable arrangement looping and live track inputs. Native exports use trusted stopped-state snapshots. Select same-pass routed stems for native projects; legacy pre-mixer stems are available for stock projects. Routed stems retain sidechain and routing context, before master inserts.");
                if ui.button("Export to new folder").clicked(){action=Some("export");}
            });
            if let Some(job)=&self.export_job{let(done,total)=job.progress();ui.label(format!("Stage-local progress {done}/{total}"));
if ui.button("Cancel export").clicked(){job.cancel();}}
            ui.label(&self.export_report);
        });
        root.collapsing("Waveform and warp markers",|ui|{
            ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.wave_import);
if ui.button("Import original WAV path").clicked(){action=Some("import-wave");}});
            egui::ComboBox::from_id_salt("wave-source").selected_text(&self.wave_source).show_ui(ui,|ui|{for id in self.project.disk.keys().filter(|id|!self.project.warps.contains_key(*id)){ui.selectable_value(&mut self.wave_source,id.clone(),id);}});
            if ui.button("Load source for selected track").clicked(){action=Some("load-wave");}
            if let (Some(thumbnail),Some(draft))=(&self.wave_thumbnail,&mut self.wave_draft){
                if self.wave_editor.show(ui,thumbnail,&mut draft.recipe){self.wave_ready=None;self.wave_epoch+=1;self.wave_job=None;}
                if let Err(e)=draft.validate(&self.project){ui.colored_label(egui::Color32::LIGHT_RED,e.to_string());}
                if ui.add_enabled(self.wave_job.is_none(),egui::Button::new("Render warp cache")).clicked(){action=Some("render-wave");}
            }
            if let Some(job)=&self.wave_job{ui.label("Rendering (editing the draft cancels this job)");
if ui.button("Cancel warp").clicked(){job.cancel();}}
            if ui.add_enabled(self.wave_ready.is_some(),egui::Button::new("Apply cache as region")).clicked(){action=Some("apply-wave");}
            ui.label("Original source retained. Disable original overlapping regions when auditioning the new region. Apply is undoable; Play auditions the prepared result.");
        });
        root.collapsing("Recording controls", |ui| {
            ui.add_enabled_ui(!self.recording, |ui| {
                ui.checkbox(
                    &mut self.input_enabled,
                    "Use default audio input (enable monitoring intentionally)",
                );
                ui.horizontal(|ui| {
                    ui.label("Input L/R channel indices");
                    for c in &mut self.input_channels {
                        ui.add(egui::DragValue::new(c).range(0..=63));
                    }
                });
                for t in &self.project.document.tracks {
                    let mut on = self.record_settings.arms.iter().any(|a| a.track == t.id);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut on, &t.id).changed() {
                            if on {
                                self.record_settings.arms.push(recording::Arm {
                                    channels: vec![],
                                    track: t.id.clone(),
                                    source: recording::Source::RenderedTrack,
                                    monitor_input: false,
                                });
                            } else {
                                self.record_settings.arms.retain(|a| a.track != t.id);
                            }
                        }
                        if let Some(a) = self
                            .record_settings
                            .arms
                            .iter_mut()
                            .find(|a| a.track == t.id)
                        {
                            egui::ComboBox::from_id_salt(("record-source", &t.id))
                                .selected_text(format!("{:?}", a.source))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut a.source,
                                        recording::Source::RenderedTrack,
                                        "Track with inserts and clips",
                                    );
                                    ui.selectable_value(
                                        &mut a.source,
                                        recording::Source::DryInput,
                                        "Dry mapped input",
                                    );
                                });
                            ui.checkbox(&mut a.monitor_input, "Monitor input through track");
                            let mut independent = !a.channels.is_empty();
                            if ui
                                .checkbox(&mut independent, "Independent hardware channels")
                                .changed()
                            {
                                a.channels = if independent { vec![0] } else { vec![] };
                            }
                            if independent {
                                let mut stereo = a.channels.len() == 2;
                                if ui.checkbox(&mut stereo, "Stereo pair").changed() {
                                    a.channels = if stereo {
                                        vec![a.channels[0], a.channels[0].saturating_add(1).min(63)]
                                    } else {
                                        vec![a.channels[0]]
                                    };
                                }
                                for channel in &mut a.channels {
                                    ui.add(
                                        egui::DragValue::new(channel)
                                            .range(0..=63)
                                            .prefix("Input (0-based) "),
                                    );
                                }
                            }
                        }
                    });
                }
                ui.checkbox(&mut self.record_settings.master, "Record master");
                ui.horizontal(|ui| {
                    ui.label("Start beat");
                    ui.add(
                        egui::DragValue::new(&mut self.record_settings.start_beat)
                            .range(0.0..=1_000_000.),
                    );
                    ui.label("Count-in quarter beats");
                    ui.add(
                        egui::DragValue::new(&mut self.record_settings.count_in_beats)
                            .range(0..=32),
                    );
                    ui.label("Pre-roll seconds");
                    ui.add(
                        egui::DragValue::new(&mut self.record_settings.pre_roll_seconds)
                            .range(0.0..=10.),
                    );
                });
                let mut punch = self.record_settings.punch_beats.is_some();
                if ui
                    .checkbox(&mut punch, "Single-pass punch (disable arrangement loop)")
                    .changed()
                {
                    self.record_settings.punch_beats = if punch { Some([0., 4.]) } else { None };
                }
                if let Some(range) = &mut self.record_settings.punch_beats {
                    ui.horizontal(|ui| {
                        for b in range {
                            ui.add(egui::DragValue::new(b).range(0.0..=1_000_000.));
                        }
                    });
                }
                ui.add(
                    egui::Slider::new(&mut self.record_settings.click_gain, 0.0..=0.25)
                        .text("Click gain"),
                );
            });
            if ui
                .checkbox(
                    &mut self.record_settings.metronome,
                    "Metronome (monitor only)",
                )
                .changed()
            {
                if let Some(p) = &self.record_progress {
                    p.set_metronome(self.record_settings.metronome);
                }
            }
            if let Some(p) = &self.record_progress {
                let p = p.status();
                ui.label(format!(
                    "Phase {} · count-in remaining {} frames · acquired {} frames · {} stems",
                    p.phase, p.count_in_remaining, p.captured_frames, p.armed_stems
                ));
            }
        });
        if let Some(c) = &self.control {
            let s = c.status();
            if let Some(writer) = &mut self.automation_writer {
                if let Ok(beat) =
                    soniclaw::transport::TempoMap::compile(&self.project.document.timeline)
                        .and_then(|m| {
                            m.beat_at_sample(s.playhead, self.project.document.sample_rate as f64)
                        })
                {
                    if let Err(e) = writer.tick(beat) {
                        self.status = e.to_string();
                    }
                }
            }
            root.label(format!(
                "Frame {} · revision {} · failures {}",
                s.playhead, s.applied_revision, s.failures
            ));
            root.horizontal(|ui| {
                for p in c.parameters() {
                    ui.vertical(|ui| {
                        let response = ui::audio_fader(ui, p, 100.);
                        if let Some(writer) = &mut self.automation_writer {
                            if writer.parameter() == p.info().id {
                                let gesture = c.gestures().get(writer.lane_index());
                                let beat = soniclaw::transport::TempoMap::compile(
                                    &self.project.document.timeline,
                                )
                                .and_then(|map| {
                                    map.beat_at_sample(
                                        s.playhead,
                                        self.project.document.sample_rate as f64,
                                    )
                                });
                                if response.changed() {
                                    if let Ok(beat) = beat {
                                        if let Err(e) =
                                            writer.input(beat, p.normalized(), response.dragged())
                                        {
                                            self.status = e.to_string();
                                        }
                                        if let Some(g) = gesture {
                                            if let Err(e) = g.set(p.normalized()) {
                                                self.status = e.to_string();
                                            }
                                            if !response.dragged() {
                                                g.end();
                                            }
                                        }
                                    }
                                }
                                if response.drag_stopped() {
                                    writer.end();
                                    if let Some(g) = gesture {
                                        g.end();
                                    }
                                }
                            }
                        }
                    });
                }
            });
        }
        root.horizontal(|ui| {
            ui.label("Project folder");
            ui.text_edit_singleline(&mut self.folder);
            if ui
                .add_enabled(self.save_job.is_none(), egui::Button::new("Save project"))
                .clicked()
            {
                action = Some("save");
            }
            if ui.button("Open folder").clicked() {
                action = Some("open");
            }
        });
        root.collapsing("Song arrangement and musical timeline", |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.play_from_beat)
                        .range(0.0..=1_000_000.)
                        .prefix("Play from beat "),
                );
                if ui.button("Locate").clicked() {
                    action = Some("locate");
                }
                ui.label("Space: Play/Stop when not typing");
            });
            ui.checkbox(
                &mut self.audio_anchor_beats,
                "Anchor audio starts to beats on tempo changes (content stays fixed seconds)",
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.tempo_beat)
                        .range(0.0..=1_000_000.)
                        .prefix("Tempo beat "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.tempo_bpm)
                        .range(20.0..=400.)
                        .prefix("BPM "),
                );
                ui.checkbox(&mut self.tempo_ramp, "Ramp to next");
                if ui.button("Apply tempo").clicked() {
                    self.timing_command = Some(arrangement::Command::Tempo(
                        soniclaw::transport::TempoPoint {
                            beat: self.tempo_beat,
                            bpm: self.tempo_bpm,
                            transition: if self.tempo_ramp {
                                soniclaw::transport::Transition::Linear
                            } else {
                                soniclaw::transport::Transition::Step
                            },
                        },
                    ));
                    action = Some("timeline-command");
                }
                if ui.button("Remove tempo beat").clicked() {
                    self.timing_command = Some(arrangement::Command::RemoveTempo(self.tempo_beat));
                    action = Some("timeline-command");
                }
            });
            for point in &self.project.document.timeline.tempo {
                if ui
                    .selectable_label(
                        self.tempo_beat == point.beat,
                        format!(
                            "Tempo beat {} · {} BPM · {:?}",
                            point.beat, point.bpm, point.transition
                        ),
                    )
                    .clicked()
                {
                    self.tempo_beat = point.beat;
                    self.tempo_bpm = point.bpm;
                    self.tempo_ramp = point.transition == soniclaw::transport::Transition::Linear;
                }
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.meter_beat)
                        .range(0.0..=1_000_000.)
                        .prefix("Meter beat "),
                );
                ui.add(egui::DragValue::new(&mut self.meter_numerator).range(1..=32));
                egui::ComboBox::from_id_salt("meter-denominator")
                    .selected_text(self.meter_denominator.to_string())
                    .show_ui(ui, |ui| {
                        for denominator in [1, 2, 4, 8, 16, 32] {
                            ui.selectable_value(
                                &mut self.meter_denominator,
                                denominator,
                                denominator.to_string(),
                            );
                        }
                    });
                if ui.button("Apply meter").clicked() {
                    self.timing_command = Some(arrangement::Command::Meter(
                        soniclaw::transport::MeterPoint {
                            beat: self.meter_beat,
                            numerator: self.meter_numerator,
                            denominator: self.meter_denominator,
                        },
                    ));
                    action = Some("timeline-command");
                }
                if ui.button("Remove meter beat").clicked() {
                    self.timing_command = Some(arrangement::Command::RemoveMeter(self.meter_beat));
                    action = Some("timeline-command");
                }
            });
            for point in &self.project.document.timeline.meters {
                if ui
                    .selectable_label(
                        self.meter_beat == point.beat,
                        format!(
                            "Meter beat {} · {}/{}",
                            point.beat, point.numerator, point.denominator
                        ),
                    )
                    .clicked()
                {
                    self.meter_beat = point.beat;
                    self.meter_numerator = point.numerator;
                    self.meter_denominator = point.denominator;
                }
            }
            ui.horizontal(|ui| {
                ui.label("Label ID");
                ui.text_edit_singleline(&mut self.label_id);
                if ui.button("New label").clicked() {
                    self.label_id.clear();
                }
                ui.text_edit_singleline(&mut self.label_name);
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.label_start)
                        .range(0.0..=1_000_000.)
                        .prefix("Start/beat "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.label_end)
                        .range(0.0..=1_000_000.)
                        .prefix("Section end "),
                );
                if ui.button("Apply marker").clicked() {
                    action = Some("marker-upsert");
                }
                if ui.button("Apply section").clicked() {
                    action = Some("section-upsert");
                }
            });
            for marker in &self.project.layout.arrangement.markers {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.label_id == marker.id,
                            format!("Marker {} · beat {}", marker.name, marker.beat),
                        )
                        .clicked()
                    {
                        self.label_id = marker.id.clone();
                        self.label_name = marker.name.clone();
                        self.label_start = marker.beat;
                    }
                    if ui.button(format!("Locate {}", marker.name)).clicked() {
                        self.play_from_beat = marker.beat;
                        action = Some("locate");
                    }
                    if ui.small_button("Delete").clicked() {
                        self.timing_command =
                            Some(arrangement::Command::RemoveMarker(marker.id.clone()));
                        action = Some("timeline-command");
                    }
                });
            }
            for section in &self.project.layout.arrangement.sections {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.label_id == section.id,
                            format!(
                                "Section {} · {}–{}",
                                section.name, section.start, section.end
                            ),
                        )
                        .clicked()
                    {
                        self.label_id = section.id.clone();
                        self.label_name = section.name.clone();
                        self.label_start = section.start;
                        self.label_end = section.end;
                    }
                    if ui.small_button("Delete").clicked() {
                        self.timing_command =
                            Some(arrangement::Command::RemoveSection(section.id.clone()));
                        action = Some("timeline-command");
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.duplicate_beat)
                        .range(0.0..=1_000_000.)
                        .prefix("Copy destination beat "),
                );
                ui.checkbox(&mut self.duplicate_automation, "Copy automation");
                if ui.button("Duplicate selected regions").clicked() {
                    action = Some("duplicate-arrangement");
                }
                if ui.button("Duplicate selected section").clicked() {
                    action = Some("duplicate-section");
                }
            });
        });
        root.collapsing("MIDI controller points", |ui| {
            let clip = self
                .project
                .document
                .tracks
                .get(self.selected)
                .and_then(|t| t.midi.iter().find(|c| c.id == self.midi_clip))
                .cloned();
            if let Some(clip) = clip {
                ui.label(format!(
                    "Selected clip: {} · {} stepped points",
                    clip.id,
                    clip.controls.len()
                ));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.control_id).prefix("Point ID "));
                    if ui.button("New point ID").clicked() {
                        self.control_id = clip
                            .controls
                            .iter()
                            .map(|p| p.id)
                            .max()
                            .map_or(0, |id| id.saturating_add(1));
                    }
                });
                ui.add(
                    egui::DragValue::new(&mut self.control_beat)
                        .range(0.0..=(clip.source_length - 1e-9).max(0.))
                        .speed(0.125)
                        .prefix("Source beat "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.control_channel)
                        .range(0..=15)
                        .prefix("Channel (0-based) "),
                );
                egui::ComboBox::from_id_salt("control-kind")
                    .selected_text(
                        ["CC", "Pitch bend", "Channel pressure", "Poly pressure"]
                            [self.control_kind.min(3) as usize],
                    )
                    .show_ui(ui, |ui| {
                        for (i, label) in ["CC", "Pitch bend", "Channel pressure", "Poly pressure"]
                            .iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut self.control_kind, i as u8, *label);
                        }
                    });
                if self.control_kind == 0 || self.control_kind == 3 {
                    ui.add(
                        egui::DragValue::new(&mut self.control_key)
                            .range(0..=127)
                            .prefix("Controller/key "),
                    );
                }
                ui.add(
                    egui::Slider::new(&mut self.control_value, 0.0..=1.0)
                        .text("Value; bend centre ≈ 0.5"),
                );
                if ui.button("Apply point").clicked() {
                    action = Some("midi-control-upsert");
                }
                if ui.button("Remove point ID").clicked() {
                    action = Some("midi-control-remove");
                }
                egui::ScrollArea::vertical()
                    .id_salt("control-list")
                    .max_height(180.)
                    .show(ui, |ui| {
                        for point in &clip.controls {
                            if ui
                                .selectable_label(
                                    self.control_id == point.id,
                                    format!(
                                        "{} · beat {:.4} · ch {} · {:?}",
                                        point.id, point.beat, point.channel, point.control
                                    ),
                                )
                                .clicked()
                            {
                                self.control_id = point.id;
                                self.control_beat = point.beat;
                                self.control_channel = point.channel;
                                match point.control {
                                    midi_edit::Control::Cc { controller, value } => {
                                        self.control_kind = 0;
                                        self.control_key = controller;
                                        self.control_value = value as f32 / 127.;
                                    }
                                    midi_edit::Control::Bend { value } => {
                                        self.control_kind = 1;
                                        self.control_value = value as f32 / 16383.;
                                    }
                                    midi_edit::Control::Pressure { value } => {
                                        self.control_kind = 2;
                                        self.control_value = value as f32 / 127.;
                                    }
                                    midi_edit::Control::PolyPressure { key, value } => {
                                        self.control_kind = 3;
                                        self.control_key = key;
                                        self.control_value = value as f32 / 127.;
                                    }
                                }
                            }
                        }
                    });
            } else {
                ui.label("Select an instrument track and MIDI clip first.");
            }
        });
        root.collapsing("Take groups and MIDI loop passes", |ui| {
            ui.checkbox(
                &mut self.midi_settings.looping,
                "Record MIDI in arrangement loop mode",
            );
            ui.add(
                egui::DragValue::new(&mut self.midi_take_index)
                    .range(0..=self.project.midi_takes.len().saturating_sub(1))
                    .prefix("Raw MIDI take index "),
            );
            if ui.button("Load saved raw MIDI take").clicked() {
                action = Some("midi-load-take");
            }
            if let Some(take) = &self.midi_last {
                if let Ok(passes) = take.passes() {
                    ui.label(format!("{} complete MIDI loop passes", passes.len()));
                    ui.add(
                        egui::DragValue::new(&mut self.midi_pass_index)
                            .range(0..=passes.len().saturating_sub(1))
                            .prefix("Pass index "),
                    );
                    if ui
                        .add_enabled(!passes.is_empty(), egui::Button::new("Use MIDI pass"))
                        .clicked()
                    {
                        action = Some("midi-choose-pass");
                    }
                }
            }
            if ui
                .add_enabled(
                    self.comp.is_some(),
                    egui::Button::new("Apply current audio comp to all take stems"),
                )
                .clicked()
            {
                action = Some("group-comp");
            }
            for (id, group) in &self.project.layout.take_groups {
                let mut active = group.active;
                if ui
                    .checkbox(
                        &mut active,
                        format!("{id}: {} linked stems", group.destinations.len()),
                    )
                    .changed()
                {
                    self.group_toggle = Some((id.clone(), active));
                    action = Some("toggle-comp-group");
                }
                if ui.button(format!("Dissolve {id} phase lock")).clicked() {
                    self.ungroup_id = Some(id.clone());
                    action = Some("ungroup-comp");
                }
            }
        });
        root.collapsing("Recording calibration",|ui|{
            let mut enabled=self.project.layout.calibration.is_some();if ui.checkbox(&mut enabled,"Apply measured/manual offsets").changed(){self.project.layout.calibration=if enabled{Some(timing::Calibration::new(self.project.document.sample_rate))}else{None};}
            if let Some(c)=&mut self.project.layout.calibration {let rate=c.sample_rate as i32;ui.label("Positive values move late recorded performances earlier; raw media is preserved.");ui.add(egui::DragValue::new(&mut c.audio_offset_frames).range(-rate..=rate).prefix("Audio frames "));ui.add(egui::DragValue::new(&mut c.midi_offset_frames).range(-rate..=rate).prefix("MIDI frames "));ui.text_edit_singleline(&mut c.label);}
            ui.label("Electrical single-impulse loopback: mono float32 WAV files at project rate. Use low output level; disconnect speakers while routing output to input.");ui.text_edit_singleline(&mut self.latency_reference);ui.text_edit_singleline(&mut self.latency_return);if ui.add_enabled(self.latency_job.is_none(),egui::Button::new("Measure impulse loopback files")).clicked(){action=Some("measure-loopback");}
        });
        root.collapsing("Recover interrupted audio WAV",|ui|{ui.label("Select an interrupted Soniclaw recording and a new WAV path. Original bytes are preserved; review the recovered prefix before import.");ui.text_edit_singleline(&mut self.salvage_source);ui.text_edit_singleline(&mut self.salvage_target);if ui.add_enabled(self.salvage_job.is_none(),egui::Button::new("Recover whole recorded frames")).clicked(){action=Some("salvage-audio");}});
        root.collapsing("Recovery and missing media",|ui|{
            ui.label(format!("Project identity: {}",self.project.project_id));
            ui.label("Pending performances are checkpointed every two seconds. Recovery requires the saved project with unchanged track/timeline or armed automation lanes.");
            if ui.button("Accept and recover pending MIDI prefix").clicked(){action=Some("recover-pending-midi");}
            if ui.button("Recover pending automation").clicked(){action=Some("recover-pending-automation");}
            let busy=self.autosave.as_ref().is_some_and(|a|a.busy());
            ui.horizontal(|ui|{
                ui.label("Recovery folder");
                if ui.add_enabled(!self.autosave_enabled&&!busy,egui::TextEdit::singleline(&mut self.recovery_folder)).changed(){self.autosave=None;self.recovery_store=None;}
                if ui.add_enabled(!self.autosave_enabled&&!busy,egui::DragValue::new(&mut self.autosave_seconds).range(1..=3600).suffix(" seconds")).changed(){self.autosave=None;}
            });
            let mut enabled=self.autosave_enabled;
if ui.checkbox(&mut enabled,"Enable periodic autosave").changed(){action=Some(if enabled{"enable-autosave"}else{"disable-autosave"});}
            ui.horizontal(|ui|{if ui.add_enabled(!busy,egui::Button::new("Checkpoint now")).clicked(){action=Some("checkpoint");}
                if ui.add_enabled(!busy,egui::Button::new("Restore latest verified snapshot")).clicked(){action=Some("restore-recovery");}});
            if self.autosave_enabled&&!self.owners.is_empty(){ui.label("Autosave waits for stopped native checkpoint ownership");}
            ui.label("Media repair inspects the Project folder above and fills missing files only; existing files are never overwritten.");
            if ui.button("Inspect project media").clicked(){action=Some("inspect-media");}
            for(i,issue)in self.resource_issues.iter().enumerate(){ui.selectable_value(&mut self.repair_selected,i,format!("{} ({:?}): {}",issue.id,issue.kind,issue.reason));}
            ui.horizontal(|ui|{ui.label("Exact replacement file");ui.text_edit_singleline(&mut self.repair_path);
if ui.add_enabled(!self.resource_issues.is_empty(),egui::Button::new("Repair selected missing media")).clicked(){action=Some("repair-media");}});
        });
        root.horizontal(|ui| {
            let mut range = self.project.document.timeline.loop_range;
            let mut enabled = range.is_some();
            let mut changed = ui.checkbox(&mut enabled, "Loop arrangement").changed();
            if changed {
                range = if enabled { Some([0., 4.]) } else { None };
            }
            if let Some(r) = &mut range {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut r[0])
                            .range(0.0..=1_000_000.)
                            .prefix("Loop start "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut r[1])
                            .range(0.0..=1_000_000.)
                            .prefix("End beat "),
                    )
                    .changed();
            }
            if changed {
                let mut timeline = self.project.document.timeline.clone();
                timeline.loop_range = range;
                self.timing_pending = Some(timeline);
                action = Some("set-timeline");
            }
        });
        root.horizontal(|ui| {
            for (i, t) in self.project.document.tracks.iter().enumerate() {
                let strip = &self.project.document.mixer.strips[i];
                let label = self
                    .project
                    .layout
                    .membership
                    .get(&t.id)
                    .and_then(|f| self.project.layout.folders.get(f))
                    .map_or_else(
                        || strip.name.clone(),
                        |folder| format!("{folder} / {}", strip.name),
                    );
                if ui.selectable_value(&mut self.selected, i, label).clicked() {
                    self.track_name = strip.name.clone();
                    self.midi_clip.clear();
                }
            }
            if ui.button("Freeze track").clicked() {
                action = Some("freeze");
            }
            if ui.button("Unfreeze").clicked() {
                action = Some("unfreeze");
            }
        });
        egui::ScrollArea::vertical().show(root, |ui| {
            if let Ok(mut draft) = self.project.edit_view() {
                if let Err(e) =
                    self.editor
                        .arrangement(ui, &mut draft.document, self.project.document.tracks.len())
                {
                    self.status = e.to_string();
                }
                if let Err(e)=self.editor.audio_tools(ui,&mut draft.document){self.status=e.to_string();}
                let clips=&self.project.document.tracks[self.selected].midi;
                if !clips.iter().any(|c|c.id==self.midi_clip){self.midi_clip=clips.first().map_or_else(String::new,|c|c.id.clone());}
                if let Some(selected)=clips.iter().find(|c|self.editor.selection.clips.contains(&c.id)){self.midi_clip=selected.id.clone();}
                let previous=self.midi_clip.clone();egui::ComboBox::from_id_salt("edit-midi-clip").selected_text(&self.midi_clip).show_ui(ui,|ui|{for c in clips{ui.selectable_value(&mut self.midi_clip,c.id.clone(),&c.id);}});
if previous!=self.midi_clip{self.editor.selection.clips.clear();self.editor.selection.clips.insert(self.midi_clip.clone());}
                if !self.midi_clip.is_empty(){if let Err(e)=self.editor.piano_roll(ui,&mut draft.document,&self.midi_clip){self.status=e.to_string();}}
                if let Err(e) = self.project.replace_edit_view(draft) {
                    self.status = e.to_string();
                }
            }
            ui.collapsing("Automation", |ui| {
                let ids = if let Some(c) = &self.control {
                    c.parameters()
                        .iter()
                        .map(|p| p.info().id.to_string())
                        .collect::<Vec<_>>()
                } else {
                    self.project
                        .document
                        .automation
                        .lanes
                        .iter()
                        .map(|l| l.parameter.clone())
                        .collect()
                };
                ui.add_enabled_ui(self.automation_writer.is_none(),|ui|{if let Err(e)=self.automation.show(ui,&mut self.project.document.automation,&ids){self.status=e.to_string();}});
            });
            ui.collapsing("Loop takes and comping",|ui|{
                for(index,path)in self.project.takes.iter().enumerate(){if ui.button(format!("Load passes from take {}",index+1)).clicked(){let load=(||->Result<CompDraft>{let manifest=soniclaw::record::multitrack::Take::read(path)?;let recipe=soniclaw::record::comp::Recipe::new(&manifest)?;let stem=manifest.tracks[0].id.clone();Ok(CompDraft{take:index,manifest,recipe,stem})})();match load{Ok(draft)=>{self.comp_editor.reset(&draft.recipe);self.comp=Some(draft);},Err(e)=>self.status=e.to_string()}}}
                for(id,saved)in &self.project.comps{if ui.button(format!("Edit saved comp {id}")).clicked(){if let Ok(manifest)=soniclaw::record::multitrack::Take::read(&self.project.takes[saved.take]){self.comp_editor.reset(&saved.recipe);self.comp=Some(CompDraft{take:saved.take,manifest,recipe:saved.recipe.clone(),stem:saved.stem.clone()});self.comp_target=id.clone();}}}
                if let Some(draft)=&mut self.comp{egui::ComboBox::from_id_salt("comp-stem").selected_text(&draft.stem).show_ui(ui,|ui|{for track in &draft.manifest.tracks{ui.selectable_value(&mut draft.stem,track.id.clone(),&track.id);}});
if let Err(e)=self.comp_editor.show(ui,&draft.manifest,&mut draft.recipe){self.status=e.to_string();}ui.text_edit_singleline(&mut self.comp_target);
if ui.button("Add / update comp track").clicked(){action=Some("apply-comp");}ui.label("Original tracks/takes are retained. Mute source tracks when auditioning the new comp.");}
            });
            ui.collapsing("Native CLAP insert", |ui| {
                ui.label("Only modules explicitly allowlisted at startup can load.");
                ui.text_edit_singleline(&mut self.plugin_path);
                ui.text_edit_singleline(&mut self.plugin_id);
                if ui.button("Add insert to selected track").clicked() {
                    action = Some("add-plugin");
                }
                for native in &mut self.owners {
                    ui.label(&native.id);
                    match self.plugin_windows.for_instance(&native.id,&native.entry){Ok(state)=>{ui.horizontal(|ui|{
                        ui.selectable_value(&mut state.mode,soniclaw_host::editor_workspace::Mode::Owned,"Host window");ui.selectable_value(&mut state.mode,soniclaw_host::editor_workspace::Mode::Floating,"Floating");
                        if ui.button("Open editor").clicked(){match native.open_editor(state){Ok(w)=>{state.reopen=true;
if !w.is_empty(){self.status=w.join("; ");}},Err(e)=>self.status=e.to_string()}}
                        if ui.button("Focus").clicked(){if let Err(e)=native.focus_editor(0){self.status=e.to_string();}}
                        if ui.button("Close editor").clicked(){native.close_editor(0);state.reopen=false;}
                    });ui.horizontal(|ui|{
                        ui.add(egui::DragValue::new(&mut state.scale).range(0.5..=4.).speed(0.05));
if ui.button("Apply scale").clicked(){if let Err(e)=native.scale_editor(0,state.scale){self.status=e.to_string();}}
                        let mut size=state.size.unwrap_or([640,480]);for dimension in &mut size{ui.add(egui::DragValue::new(dimension).range(64..=8192));}state.size=Some(size);
                        if ui.button("Resize editor").clicked(){if let Err(e)=native.resize_editor(0,size){self.status=e.to_string();}}
                    });},Err(e)=>{ui.label(e.to_string());}}
                    let params = native.parameters(0).unwrap_or(&[]).to_vec();
                    ui.label("Native editor gestures update selected host controls, undo and armed automation writes.");
                    ui.label(format!("Automation failures: {}",native.automation_failures.load(std::sync::atomic::Ordering::Acquire)));
                    for p in params {
                        if p.flags & soniclaw_host::studio::READ_ONLY != 0 {ui.label(format!("{}: {} (read-only)",p.name,p.value));continue;}
                        if native.controls.iter().any(|c|c.native_id==p.id){ui.label(format!("{}: {}",p.name,p.value));continue;}
                        let mut value = p.value;
                        if ui
                            .add(egui::Slider::new(&mut value, p.min..=p.max).text(&p.name))
                            .changed()
                        {
                            if let Err(e) = native.set_parameter(0, p.id, value) {
                                self.status = e.to_string();
                            }
                        }
                    }
                }
            });
            ui.label(format!(
                "{} saved takes · {} frozen tracks",
                self.project.takes.len(),
                self.project.freezes.len()
            ));
        });
        self.sync_controls();
        if pointer_down
            && !self.history.stats().is_ok_and(|s| s.grouped)
            && !history::equivalent(&previous, &self.project).unwrap_or(false)
        {
            if let Err(e) = self.history.begin_group(&previous, "Studio gesture") {
                self.status = e.to_string();
            }
        }
        if let Err(e) = self.record_project(previous, "Studio edit") {
            self.status = e.to_string();
        }
        if !pointer_down
            && !self.native_gestures.active()
            && self.history.stats().is_ok_and(|s| s.grouped)
        {
            if let Err(e) = self.history.finish_group(&mut self.project) {
                self.status = e.to_string();
            }
        }
        if let Some(action) = action {
            let old_status = self.status.clone();
            match self.act(action) {
                Ok(()) => {
                    if self.status == old_status {
                        self.status = format!("{action} completed");
                    }
                }
                Err(e) => self.status = e.to_string(),
            };
        }
        root.label(&self.status);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
impl eframe::App for Studio {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(root);
    }
}
impl Drop for Studio {
    fn drop(&mut self) {
        let _ = self.stop();
        self.manager.stop();
    }
}
pub fn run(args: &[String]) -> Result<()> {
    let folder = args.first().map_or("--new", String::as_str);
    let project = if folder == "--new" {
        workspace::blank("Untitled Song".into(), 48000, 256)?
    } else {
        Project::open(Path::new(folder)).map_err(|e| e.to_string())?
    };
    let trusted = args
        .get(1..)
        .unwrap_or(&[])
        .iter()
        .map(|p| PathBuf::from(p).canonicalize())
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let prefs = DevicePreferences {
        sample_rate: Some(project.document.sample_rate),
        channels: Some(2),
        max_internal_frames: project.document.max_block_frames,
        reconnect: false,
        ..Default::default()
    };
    let manager = DeviceManager::new(NativeOutputBackend, prefs).map_err(|e| e.to_string())?;
    let app = Studio {
        workspace_command: None,
        new_project_open: false,
        new_project_name: "Untitled Song".into(),
        new_project_rate: 48000,
        new_project_block: 256,
        track_name: "Track".into(),
        track_folder_name: "Folder".into(),
        midi_clip: String::new(),
        midi_connection: None,
        midi_keyboard: None,
        midi_held: Default::default(),
        midi_ports: vec![],
        midi_port: None,
        midi_settings: Default::default(),
        midi_recorder: None,
        midi_last: None,
        midi_folder: String::new(),
        midi_status: String::new(),
        media_plan: None,
        media_audit_job: None,
        media_copy_job: None,
        media_folder: String::new(),
        media_status: String::new(),
        automation_writer: None,
        automation_parameter: String::new(),
        automation_mode: soniclaw::automation::Mode::Touch,
        automation_grid: 0.0625,
        isolation_tools: Default::default(),
        native_buses: Default::default(),
        native_gestures: Default::default(),
        native_tools: Default::default(),
        plugin_windows: load_plugin_windows(Path::new(folder))?,
        routing_draft: None,
        export_settings: Default::default(),
        export_folder: String::new(),
        export_job: None,
        export_report: String::new(),
        jobs: jobs::Scheduler::new(Default::default()).expect("start studio background scheduler"),
        job_epoch: 0,
        wave_epoch: 0,
        wave_editor: Default::default(),
        wave_loader: None,
        wave_thumbnail: None,
        wave_draft: None,
        wave_job: None,
        wave_ready: None,
        wave_source: String::new(),
        wave_import: String::new(),
        record_settings: recording::Settings::for_document(&project.document),
        record_progress: None,
        input_enabled: false,
        input_bus: None,
        input_channels: [0, 0],
        autosave: None,
        autosave_enabled: false,
        recovery_store: None,
        recovery_folder: String::new(),
        autosave_seconds: 30,
        resource_issues: vec![],
        elastic: ElasticTools::default(),
        mix: MixTools::default(),
        cues: CueTools::default(),
        tools: Tools {
            root: 60,
            ..Default::default()
        },
        play_from_beat: 0.,
        timing_command: None,
        timing_pending: None,
        audio_anchor_beats: false,
        tempo_beat: 0.,
        tempo_bpm: 120.,
        tempo_ramp: false,
        meter_beat: 0.,
        meter_numerator: 4,
        meter_denominator: 4,
        label_id: String::new(),
        label_name: "Marker".into(),
        label_start: 0.,
        label_end: 4.,
        duplicate_beat: 4.,
        duplicate_automation: false,
        control_id: 0,
        control_beat: 0.,
        control_channel: 0,
        control_kind: 0,
        control_key: 64,
        control_value: 0.,
        midi_take_index: 0,
        midi_pass_index: 0,
        group_toggle: None,
        ungroup_id: None,
        salvage_source: String::new(),
        salvage_target: String::new(),
        salvage_job: None,
        latency_reference: String::new(),
        latency_return: String::new(),
        latency_job: None,
        save_job: None,
        saved_baseline: None,
        pending_job: None,
        pending_at: Instant::now(),
        repair_selected: 0,
        repair_path: String::new(),
        history: Default::default(),
        freeze_job: None,
        project,
        manager,
        control: None,
        owners: vec![],
        trusted,
        recorder: None,
        recording: false,
        record_path: None,
        selected: 0,
        editor: Default::default(),
        comp_editor: Default::default(),
        comp: None,
        comp_target: "comp".into(),

        automation: Default::default(),
        folder: if folder == "--new" {
            "SoniclawProject".into()
        } else {
            format!("{folder}-copy")
        },
        plugin_path: String::new(),
        plugin_id: String::new(),
        status: "Ready; Play opens the selected project rate on the default device".into(),
        serial: 0,
        started: Instant::now(),
    };
    eframe::run_native(
        "Soniclaw Reference Studio",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1180., 850.]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(app))),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_ui_and_project_actions_work_without_opening_a_device() {
        let root =
            std::env::temp_dir().join(format!("soniclaw-reference52-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let (mut d, a) = soniclaw_studio_demo::fixture();
        d.timeline.loop_range = None;
        d.tracks[1].midi_input = false;
        let project = Project::new("UI test".into(), d, a, Default::default()).unwrap();
        let mut studio = Studio {
            workspace_command: None,
            new_project_open: false,
            new_project_name: "Untitled Song".into(),
            new_project_rate: 48000,
            new_project_block: 256,
            track_name: "Track".into(),
            track_folder_name: "Folder".into(),
            midi_clip: String::new(),
            midi_connection: None,
            midi_keyboard: None,
            midi_held: Default::default(),
            midi_ports: vec![],
            midi_port: None,
            midi_settings: Default::default(),
            midi_recorder: None,
            midi_last: None,
            midi_folder: String::new(),
            midi_status: String::new(),
            media_plan: None,
            media_audit_job: None,
            media_copy_job: None,
            media_folder: String::new(),
            media_status: String::new(),
            automation_writer: None,
            automation_parameter: String::new(),
            automation_mode: soniclaw::automation::Mode::Touch,
            automation_grid: 0.0625,
            isolation_tools: Default::default(),
            native_buses: Default::default(),
            native_gestures: Default::default(),
            native_tools: Default::default(),
            plugin_windows: Default::default(),
            routing_draft: None,
            export_settings: Default::default(),
            export_folder: String::new(),
            export_job: None,
            export_report: String::new(),
            jobs: jobs::Scheduler::new(Default::default())
                .expect("start studio background scheduler"),
            job_epoch: 0,
            wave_epoch: 0,
            wave_editor: Default::default(),
            wave_loader: None,
            wave_thumbnail: None,
            wave_draft: None,
            wave_job: None,
            wave_ready: None,
            wave_source: String::new(),
            wave_import: String::new(),
            record_settings: recording::Settings::for_document(&project.document),
            record_progress: None,
            input_enabled: false,
            input_bus: None,
            input_channels: [0, 0],
            autosave: None,
            autosave_enabled: false,
            recovery_store: None,
            recovery_folder: String::new(),
            autosave_seconds: 30,
            resource_issues: vec![],
            elastic: ElasticTools::default(),
            mix: MixTools::default(),
            cues: CueTools::default(),
            tools: Tools {
                root: 60,
                ..Default::default()
            },
            play_from_beat: 0.,
            timing_command: None,
            timing_pending: None,
            audio_anchor_beats: false,
            tempo_beat: 0.,
            tempo_bpm: 120.,
            tempo_ramp: false,
            meter_beat: 0.,
            meter_numerator: 4,
            meter_denominator: 4,
            label_id: String::new(),
            label_name: "Marker".into(),
            label_start: 0.,
            label_end: 4.,
            duplicate_beat: 4.,
            duplicate_automation: false,
            control_id: 0,
            control_beat: 0.,
            control_channel: 0,
            control_kind: 0,
            control_key: 64,
            control_value: 0.,
            midi_take_index: 0,
            midi_pass_index: 0,
            group_toggle: None,
            ungroup_id: None,
            salvage_source: String::new(),
            salvage_target: String::new(),
            salvage_job: None,
            latency_reference: String::new(),
            latency_return: String::new(),
            latency_job: None,
            save_job: None,
            saved_baseline: None,
            pending_job: None,
            pending_at: Instant::now(),
            repair_selected: 0,
            repair_path: String::new(),
            history: Default::default(),
            freeze_job: None,
            project,
            manager: DeviceManager::new(NativeOutputBackend, Default::default()).unwrap(),
            control: None,
            owners: vec![],
            trusted: BTreeSet::new(),
            recorder: None,
            recording: false,
            record_path: None,
            selected: 1,
            editor: Default::default(),
            comp_editor: Default::default(),
            comp: None,
            comp_target: "comp".into(),

            automation: Default::default(),
            folder: root.join("saved").to_string_lossy().into_owned(),
            plugin_path: String::new(),
            plugin_id: String::new(),
            status: String::new(),
            serial: 0,
            started: Instant::now(),
        };
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(Default::default(), |ui| studio.show(ui));
        output.textures_delta.clear();
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Record stems")));
        assert!(!studio.manager.is_running());
        studio
            .history
            .edit(&mut studio.project, "Mixer and region", |p| {
                p.document.mixer.strips[0].gain = 0.25;
                p.document.tracks[0].audio[0].start_frame = 128;
                Ok(())
            })
            .unwrap();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| studio.show(ui));
        output.textures_delta.clear();
        assert_eq!(studio.project.document.tracks[0].audio[0].start_frame, 0);
        studio.act("redo-project").unwrap();
        assert_eq!(studio.project.document.mixer.strips[0].gain, 0.25);
        studio.history.clear().unwrap();
        studio.act("freeze").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while studio.freeze_job.is_some() {
            studio.poll_freeze();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(studio.project.freezes.len(), 1);
        studio.act("checkpoint").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while studio.autosave.as_ref().unwrap().busy() {
            studio.poll_recovery();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        studio.project.name = "Uncheckpointed change".into();
        studio.act("restore-recovery").unwrap();
        assert_ne!(studio.project.name, "Uncheckpointed change");
        studio.act("save").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while studio.save_job.is_some() {
            studio.poll_session();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        studio.act("open").unwrap();
        assert_eq!(studio.project.freezes.len(), 1);
        studio.selected = 1;
        studio.act("unfreeze").unwrap();
        assert!(studio.project.document.tracks[1].instrument.is_some());
        studio.history.clear().unwrap();
        let settings = midi_recording::Settings {
            track: studio.project.document.tracks[1].id.clone(),
            clip: "headless-midi".into(),
            ..Default::default()
        };
        let mut armed =
            midi_recording::prepare(&studio.project, &mut Builtins, settings, 64).unwrap();
        let mut pcm = vec![0.; 512];
        armed.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
            &ProcessContext {
                events: &[
                    Event {
                        frame: 7,
                        message: MidiMessage::NoteOn {
                            channel: 0,
                            note: 60,
                            velocity: 100,
                        },
                    },
                    Event {
                        frame: 90,
                        message: MidiMessage::NoteOff {
                            channel: 0,
                            note: 60,
                            velocity: 0,
                        },
                    },
                ],
                transport: Default::default(),
            },
        );
        drop(armed.processor);
        studio.midi_recorder = Some(armed.recorder);
        studio.midi_folder = root.join("midi-take").to_string_lossy().into_owned();
        let before = studio.project.document.tracks[1].midi.len();
        studio.act("midi-finish").unwrap();
        assert_eq!(studio.project.document.tracks[1].midi.len(), before + 1);
        assert!(root.join("midi-take/midi-take.json").exists());
        assert!(root.join("midi-take/performance.mid").exists());
        studio.act("undo-project").unwrap();
        assert_eq!(studio.project.document.tracks[1].midi.len(), before);
        studio.act("redo-project").unwrap();
        assert_eq!(studio.project.document.tracks[1].midi.len(), before + 1);
        studio.midi_clip = "headless-midi".into();
        studio.control_id = 900;
        studio.control_beat = 0.001;
        studio.control_kind = 1;
        studio.control_value = 0.7;
        studio.act("midi-control-upsert").unwrap();
        assert_eq!(
            studio.project.document.tracks[1]
                .midi
                .iter()
                .find(|c| c.id == "headless-midi")
                .unwrap()
                .controls
                .len(),
            1
        );
        studio.act("undo-project").unwrap();
        assert!(studio.project.document.tracks[1]
            .midi
            .iter()
            .find(|c| c.id == "headless-midi")
            .unwrap()
            .controls
            .is_empty());
        studio.act("redo-project").unwrap();
        studio.label_id.clear();
        studio.label_name = "Verse".into();
        studio.label_start = 2.;
        studio.act("marker-upsert").unwrap();
        assert_eq!(studio.project.layout.arrangement.markers.len(), 1);
        studio.act("undo-project").unwrap();
        assert!(studio.project.layout.arrangement.markers.is_empty());
        studio.act("redo-project").unwrap();
        // Stop must snapshot live controls before placing newly finalized input regions.
        let take_path = root.join("headless-audio");
        let settings = recording::Settings {
            arms: vec![recording::Arm {
                track: studio.project.document.tracks[0].id.clone(),
                source: recording::Source::DryInput,
                monitor_input: false,
                channels: vec![],
            }],
            ..Default::default()
        };
        let mut audio = recording::prepare(
            &studio.project,
            &mut Builtins,
            settings,
            &take_path,
            "headless-audio",
            64,
        )
        .unwrap();
        std::fs::write(
            take_path.join("recording-schedule.json"),
            serde_json::to_vec(&audio.schedule).unwrap(),
        )
        .unwrap();
        pcm.fill(0.125);
        audio.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        drop(audio.processor);
        let previous_regions = studio.project.document.tracks[0].streamed.len();
        studio.control = Some(audio.controller);
        studio.recorder = Some(audio.recorder);
        studio.record_progress = Some(audio.progress);
        studio.record_path = Some(take_path);
        studio.recording = true;
        studio.act("stop").unwrap();
        studio.sync_controls();
        assert_eq!(
            studio.project.document.tracks[0].streamed.len(),
            previous_regions + 1
        );
        studio.selected = 1;
        studio.tools.synth = 2;
        studio.act("tools-synth").unwrap();
        assert_eq!(
            studio.project.document.tracks[1]
                .instrument
                .as_ref()
                .unwrap()
                .kind,
            "stock.synth"
        );
        studio.tools.effect = 0;
        studio.act("tools-effect").unwrap();
        let count = studio.project.document.tracks[1].inserts.len();
        studio.tools.processor = studio.project.document.tracks[1]
            .inserts
            .last()
            .unwrap()
            .id
            .clone();
        studio.act("tools-insert-remove").unwrap();
        assert_eq!(studio.project.document.tracks[1].inserts.len(), count - 1);
        studio.act("undo-project").unwrap();
        assert_eq!(studio.project.document.tracks[1].inserts.len(), count);
        let elastic_source = root.join("elastic-original.wav");
        let mut writer = hound::WavWriter::create(
            &elastic_source,
            hound::WavSpec {
                channels: 1,
                sample_rate: studio.project.document.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..2048 {
            writer.write_sample(0.125f32).unwrap();
        }
        writer.finalize().unwrap();
        studio
            .project
            .disk
            .insert("elastic-original".into(), elastic_source);
        studio.elastic.source = "elastic-original".into();
        studio.selected = 0;
        studio.act("elastic-add").unwrap();
        assert_eq!(studio.elastic.group.as_ref().unwrap().members.len(), 1);
        let before = studio.elastic.group.clone();
        assert!(studio.act("elastic-add").is_err());
        assert_eq!(studio.elastic.group, before);
        studio.act("elastic-new").unwrap();
        assert!(studio.elastic.group.is_none());
        studio.act("mix-load").unwrap();
        studio.mix.name = "Drum submix".into();
        studio.act("mix-add-bus").unwrap();
        studio.act("mix-bus-effect").unwrap();
        studio.act("mix-master-effect").unwrap();
        studio.act("mix-apply").unwrap();
        assert_eq!(studio.project.document.mix_routing.buses.len(), 1);
        assert_eq!(studio.project.document.mix_routing.master.inserts.len(), 1);
        studio.act("undo-project").unwrap();
        assert!(studio.project.document.mix_routing.is_empty());
        studio.act("redo-project").unwrap();
        assert_eq!(studio.project.document.mix_routing.buses.len(), 1);
        studio.act("cues-load").unwrap();
        studio.cues.name = "Performer".into();
        studio.act("cues-add").unwrap();
        studio.cues.source = studio.project.document.tracks[0].id.clone();
        studio.act("cues-send-add").unwrap();
        studio.act("cues-apply").unwrap();
        assert_eq!(studio.project.document.cues.len(), 1);
        studio.act("undo-project").unwrap();
        assert!(studio.project.document.cues.is_empty());
        studio.act("redo-project").unwrap();
        assert_eq!(studio.project.document.cues.len(), 1);
        // Policy/history/trust checks execute no native module or helper.
        let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
        studio.project.document.tracks[0].inserts.push(Descriptor{id:"ui.isolated".into(),kind:"vst3".into(),state:serde_json::json!({"path":exe,"class_id":"53434C5748535450524F543100000001","state_mode":"parameters"})});
        studio.native_tools.instance = "ui.isolated".into();
        studio.isolation_tools.helper = exe.to_string_lossy().into();
        assert!(studio.isolation_action("enable").is_err());
        studio.isolation_tools.approve = true;
        studio.isolation_action("enable").unwrap();
        assert!(studio.trusted.contains(&exe));
        studio.act("undo-project").unwrap();
        assert!(native_state::descriptors(&studio.project.document)
            .find(|d| d.id == "ui.isolated")
            .unwrap()
            .state
            .get("isolation")
            .is_none());
        studio.act("redo-project").unwrap();
        for _ in 0..3 {
            studio.isolation_action("recover").unwrap();
        }
        assert!(studio.isolation_action("recover").is_err());
        studio.isolation_action("approve").unwrap();
        assert_eq!(
            native_state::descriptors(&studio.project.document)
                .find(|d| d.id == "ui.isolated")
                .unwrap()
                .state["isolation_attempts"],
            3
        );
        studio.isolation_action("disable").unwrap();
        drop(studio);
        std::fs::remove_dir_all(root).unwrap();
    }
}

include!("tools_ui.rs");

include!("elastic_ui.rs");

include!("mix_ui.rs");

include!("cues_ui.rs");

include!("native_ui.rs");

impl Studio {
    fn service_native_edits(&mut self, finish: bool) -> Result<()> {
        let mut events = Vec::new();
        for owner in &mut self.owners {
            owner.collect_editor_edits()?;
            if finish {
                owner.finish_editor_gestures()?;
            }
            events.extend(owner.take_editor_edits());
            if events.len() > 4096 {
                return Err("too many native editor events in one GUI cycle".into());
            }
        }
        let beat = self
            .control
            .as_ref()
            .map(|c| c.status().playhead)
            .map(|s| {
                soniclaw::transport::TempoMap::compile(&self.project.document.timeline)
                    .and_then(|m| m.beat_at_sample(s, self.project.document.sample_rate as f64))
            })
            .transpose()?
            .unwrap_or(0.);
        for event in events {
            self.native_gestures.apply(
                &event,
                &mut self.project,
                self.control.as_ref(),
                self.automation_writer.as_mut(),
                &mut self.history,
                beat,
            )?;
        }
        Ok(())
    }
}

include!("native_bus_ui.rs");

include!("isolation_ui.rs");
