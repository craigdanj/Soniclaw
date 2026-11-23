#[path = "panels.rs"]
mod panels;
use soniclaw::{
    device::{event_channel, EventSender, Telemetry},
    dsp::modulation::*,
    prelude::*,
    state::*,
    ui::egui,
};
use soniclaw_workbench::{default_root, preset, render, Model, Preview};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
const ACCENT: egui::Color32 = egui::Color32::from_rgb(84, 222, 190);
struct Packet {
    samples: [f32; 512],
    frames: usize,
    rate: f64,
    cpu: f32,
    latency: usize,
    position: u64,
}
struct LiveProcessor {
    rack: Option<soniclaw::graph::live::LiveGraph>,
    pending: Option<Box<dyn AudioProcessor>>,
    control_sender: Option<mpsc::SyncSender<soniclaw::graph::live::GraphController>>,
    stats: Arc<soniclaw::diagnostics::CallbackStats>,
    sender: rtrb::Producer<Packet>,
    sr: f64,
    position: u64,
    test_signal: bool,
    tempo: Arc<Parameter>,
}
impl AudioProcessor for LiveProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if let Some(p) = self.pending.take() {
            let (control, rack) = soniclaw::graph::live::graph_pair(p, s, 2048, 512)?;
            self.rack = Some(rack);
            self.control_sender
                .take()
                .ok_or(Error::NotPrepared)?
                .send(control)
                .map_err(|_| Error::NotPrepared)?;
        } else {
            self.rack.as_mut().ok_or(Error::NotPrepared)?.prepare(s)?;
        }
        self.sr = s.sample_rate;
        self.position = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let start = Instant::now();
        let ch = b.channels();
        let frames = b.frames();
        if self.test_signal {
            for (i, f) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
                let x = (std::f64::consts::TAU * 220.0 * (self.position + i as u64) as f64
                    / self.sr)
                    .sin() as f32
                    * 0.12;
                f.fill(x);
            }
        }
        let Some(rack) = &mut self.rack else {
            b.clear();
            return;
        };
        rack.process(
            b,
            &ProcessContext {
                events: ctx.events,
                transport: Transport {
                    sample_position: self.position,
                    tempo: self.tempo.get() as f64,
                    playing: true,
                },
            },
        );
        self.position += frames as u64;
        let cpu = (start.elapsed().as_secs_f64() / (frames.max(1) as f64 / self.sr) * 100.0) as f32;
        self.stats.record(
            frames,
            self.sr,
            start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            b.samples().iter().filter(|x| !x.is_finite()).count(),
        );
        let mut packet = Packet {
            samples: [0.0; 512],
            frames: frames.min(256),
            rate: self.sr,
            cpu,
            latency: rack.latency_frames(),
            position: self.position,
        };
        for (i, f) in b.samples().chunks_exact(ch).take(256).enumerate() {
            packet.samples[i * 2] = f[0];
            packet.samples[i * 2 + 1] = f[if ch > 1 { 1 } else { 0 }];
        }
        let _ = self.sender.push(packet);
    }
    fn reset(&mut self) {
        if let Some(r) = &mut self.rack {
            r.reset();
        }
        self.position = 0;
    }
    fn latency_frames(&self) -> usize {
        self.rack.as_ref().map_or(0, |r| r.latency_frames())
    }
}
#[derive(Clone, Copy)]
enum Action {
    Preset(usize),
    Preview,
    Play,
    Stop,
    Save,
    Load,
    Bundle,
    SingleBundle,
    ImportBundle,
    Recover,
    Checkpoint,
    Undo,
    Redo,
    Move(usize, isize),
    Remove(usize),
    Add(ProcessorKind),
    AddCustom(&'static str),
    Routes,
    Asset,
    GraphApply,
    SamplesApply,
    MidiApply,
    VoiceApply,
    Audit,
    HostScan,
    HostInspect,
    HostRender,
}
type TransientResult = Result<(String, Vec<usize>, usize), String>;
struct Workbench {
    vst3_direct: bool,
    vst3_musical: bool,
    vst3_requested: bool,
    vst3_path: String,
    vst3_products: Vec<soniclaw::host::formats::Product>,
    vst3_selected: usize,
    vst3_live: Option<soniclaw::vst3::Instance>,
    vst3_parameters: Vec<soniclaw::vst3::Parameter>,
    vst3_state: Option<soniclaw::vst3::State>,
    vst3_state_path: String,
    timeline_draft: Option<soniclaw::transport::Timeline>,
    host_live: Option<soniclaw::host::live::LiveRack>,
    host_session: Option<soniclaw::host::session::Session>,
    host_mapping: Vec<usize>,
    host_missing: Vec<soniclaw::host::session::Missing>,
    host_warnings: Vec<String>,
    host_requested: bool,
    host_direct: bool,
    host_buffer_blocks: usize,
    host_editor_scale: f64,
    host_editor_size: [u32; 2],
    host_chain: Vec<soniclaw::host::live::RackEntry>,
    host_node: usize,
    automation_editor: soniclaw::automation::ui::Editor,
    automation_draft: Option<soniclaw::automation::Document>,
    recording_path: String,
    record_dry: bool,
    sfz_path: String,
    transient_worker: Option<mpsc::Receiver<TransientResult>>,
    transient_ready: Option<(String, Vec<usize>, usize)>,
    browser_preview_id: Option<String>,
    browser_missing: Vec<String>,
    catalog: Option<soniclaw::catalog::PresetCatalog>,
    browser: soniclaw::catalog::browser::PresetBrowser,
    browser_root: String,
    browser_edit_id: String,
    browser_name: String,
    browser_tags: String,
    browser_bank: String,
    browser_asset_root: String,
    browser_asset_id: String,
    browser_asset_path: String,
    audition: Option<Arc<soniclaw::io::AudioFile>>,
    loader: soniclaw::project_loading::AsyncProjectLoader,
    loading: bool,
    autosave: bool,
    autosave_worker: Option<soniclaw::project_loading::AutosaveWorker>,
    autosave_at: Instant,
    recovery_directory: String,
    single_bundle: String,
    import_directory: String,
    storage_worker: Option<mpsc::Receiver<Result<String, String>>>,
    graph_control: Option<soniclaw::graph::live::GraphController>,
    graph_editor: soniclaw::ui::GraphEditor,
    graph_draft: Option<ProjectState>,
    sample_editor: soniclaw::ui::SampleMapEditor,
    sample_draft: Option<ProjectState>,
    sample_wave: Vec<f32>,
    sample_frames: usize,
    sample_cache: String,
    midi_draft: soniclaw::midi::performance::PerformanceConfig,
    voice_draft: Option<(String, soniclaw::dsp::voice_mod::VoiceModulationConfig)>,
    live_stats: Arc<soniclaw::diagnostics::CallbackStats>,
    audit_worker: Option<mpsc::Receiver<Result<soniclaw::diagnostics::AuditReport, String>>>,
    audit_report: Option<soniclaw::diagnostics::AuditReport>,
    host_path: String,
    host_products: Vec<(PathBuf, soniclaw::host::PluginInfo)>,
    host_selected: usize,
    host_parameters: Vec<soniclaw::host::ParameterInfo>,
    host_state: Option<(String, Vec<u8>)>,

    model: Model,
    device_manager: Option<soniclaw::device::DeviceManager>,
    device_draft: soniclaw::device::DevicePreferences,
    device_catalog: soniclaw::device::OutputInventory,
    device_panel: soniclaw::device::settings::DeviceSettingsPanel,
    device_settings_path: PathBuf,
    device_clock: Instant,
    pending_control: Option<mpsc::Receiver<soniclaw::graph::live::GraphController>>,
    sender: Option<EventSender>,
    receiver: Option<rtrb::Consumer<Packet>>,
    worker: Option<mpsc::Receiver<Result<Preview, String>>>,
    preview_pending: bool,
    preview: Option<Preview>,
    selected: usize,
    preset_index: usize,
    tab: usize,
    status: String,
    path: String,
    bundle: String,
    asset_path: String,
    route_draft: ModulationConfig,
    tempo: Arc<Parameter>,
    cpu: f32,
    rate: f64,
    latency: usize,
    wave: Vec<f32>,
    spectrum: Vec<f32>,
    peak: f32,
    analysis_ring: [f32; 1024],
    analysis_cursor: usize,
    analyzer: soniclaw::dsp::advanced::spectral::SpectrumAnalyzer,
    held: [bool; 128],
    screenshot: Option<PathBuf>,
    screenshot_requested: bool,
    frames: u64,
}
impl Workbench {
    fn new(
        screenshot: Option<PathBuf>,
        preset_index: usize,
        tab: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let root = default_root();
        let model = Model::new(preset_index, root.clone())?;
        let route_draft = model.snapshot().modulation.clone().unwrap_or_default();
        let mut app = Self {
            vst3_direct: false,
            vst3_musical: true,
            vst3_requested: false,
            vst3_path: String::new(),
            vst3_products: vec![],
            vst3_selected: 0,
            vst3_live: None,
            vst3_parameters: vec![],
            vst3_state: None,
            vst3_state_path: root.join("hosted-vst3.json").display().to_string(),
            timeline_draft: None,
            host_live: None,
            host_session: None,
            host_mapping: vec![],
            host_missing: vec![],
            host_warnings: vec![],
            host_requested: false,
            host_direct: true,
            host_buffer_blocks: 4,
            host_editor_scale: 1.0,
            host_editor_size: [640, 480],
            host_chain: vec![],
            host_node: 0,
            automation_editor: Default::default(),
            automation_draft: None,
            recording_path: root.join("take.wav").display().to_string(),
            record_dry: false,
            sfz_path: root.join("instrument.sfz").display().to_string(),
            transient_worker: None,
            transient_ready: None,
            browser_preview_id: None,
            browser_missing: vec![],
            catalog: None,
            browser: Default::default(),
            browser_root: root.join("preset-catalog").display().to_string(),
            browser_edit_id: String::new(),
            browser_name: String::new(),
            browser_tags: String::new(),
            browser_bank: String::new(),
            browser_asset_root: String::new(),
            browser_asset_id: String::new(),
            browser_asset_path: String::new(),
            audition: None,
            loader: soniclaw::project_loading::AsyncProjectLoader::with_factory(|s, root| {
                s.validate(soniclaw_workbench::APP_ID)?;
                soniclaw_workbench::create_plugin(
                    soniclaw_workbench::app_descriptor(s),
                    s.clone(),
                    root,
                )
            }),
            loading: false,
            autosave: false,
            autosave_worker: None,
            autosave_at: Instant::now(),
            recovery_directory: root.join("workbench-recovery").display().to_string(),
            single_bundle: root.join("workbench.soniclaw").display().to_string(),
            import_directory: root.join("imported-project").display().to_string(),
            storage_worker: None,
            graph_control: None,
            graph_editor: Default::default(),
            graph_draft: None,
            sample_editor: Default::default(),
            sample_draft: None,
            sample_wave: vec![],
            sample_frames: 0,
            sample_cache: String::new(),
            midi_draft: model.snapshot().midi.unwrap_or_default(),
            voice_draft: None,
            live_stats: Arc::new(Default::default()),
            audit_worker: None,
            audit_report: None,
            host_path: root
                .join("../../target/debug")
                .join(if cfg!(target_os = "windows") {
                    "soniclaw_workbench.dll"
                } else if cfg!(target_os = "macos") {
                    "libsoniclaw_workbench.dylib"
                } else {
                    "libsoniclaw_workbench.so"
                })
                .display()
                .to_string(),
            host_products: vec![],
            host_selected: 0,
            host_parameters: vec![],
            host_state: None,

            model,
            device_manager: Some(soniclaw::device::DeviceManager::new(
                soniclaw::device::NativeOutputBackend,
                soniclaw::device::DevicePreferences::load(root.join("audio-settings.json"))
                    .unwrap_or_default(),
            )?),
            device_draft: soniclaw::device::DevicePreferences::load(
                root.join("audio-settings.json"),
            )
            .unwrap_or_default(),
            device_catalog: Default::default(),
            device_panel: Default::default(),
            device_settings_path: root.join("audio-settings.json"),
            device_clock: Instant::now(),
            pending_control: None,
            sender: None,
            receiver: None,
            worker: None,
            preview_pending: false,
            preview: None,
            selected: if preset_index == 0 { 1 } else { 0 },
            preset_index,
            tab,
            status: "Ready · no audio device is open".into(),
            path: root.join("workbench-preset.json").display().to_string(),
            bundle: root.join("exported-project").display().to_string(),
            asset_path: "assets/grain-source.wav".into(),
            route_draft,
            tempo: Arc::new(Parameter::new(ParameterInfo {
                id: "workbench.tempo",
                name: "Tempo",
                min: 20.0,
                max: 400.0,
                default: 120.0,
                unit: "BPM",
            })?),
            cpu: 0.0,
            rate: 48000.0,
            latency: 0,
            wave: vec![],
            spectrum: vec![],
            peak: 0.0,
            analysis_ring: [0.0; 1024],
            analysis_cursor: 0,
            analyzer: soniclaw::dsp::advanced::spectral::SpectrumAnalyzer::new(
                1024,
                soniclaw::dsp::advanced::spectral::WindowKind::Hann,
            )?,
            held: [false; 128],
            screenshot,
            screenshot_requested: false,
            frames: 0,
        };
        app.preview();
        Ok(app)
    }
    fn stop(&mut self) {
        self.automation_editor.cancel_gesture();
        if let Some(s) = &self.sender {
            s.all_notes_off();
        }
        self.sender = None;
        if let Some(manager) = &mut self.device_manager {
            manager.stop();
        }
        self.vst3_requested = false;
        if let Some(mut vst) = self.vst3_live.take() {
            let _ = vst.stop();
            if let Ok(state) = vst.save_state() {
                self.vst3_state = Some(state);
            }
        }
        self.host_requested = false;
        self.host_live = None;
        self.pending_control = None;
        self.receiver = None;
        self.graph_control = None;
        self.held.fill(false);
    }
    fn preview(&mut self) {
        self.browser_preview_id = None;
        if self.worker.is_some() {
            self.preview_pending = true;
            return;
        }
        let state = self.model.snapshot();
        let root = self.model.root.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = tx.send(render(state, root, 2.0));
        });
        self.worker = Some(rx);
        self.status = "Rendering a two-second preview on a worker…".into();
    }
    fn device_running(&self) -> bool {
        self.device_manager.as_ref().is_some_and(|m| m.is_running())
    }
    fn make_output_engine(
        &mut self,
        plan: &soniclaw::device::OutputPlan,
        telemetry: Arc<Telemetry>,
    ) -> Result<soniclaw::device::OutputEngine, soniclaw::device::DeviceError> {
        self.sender = None;
        self.receiver = None;
        self.graph_control = None;
        self.pending_control = None;
        self.held.fill(false);
        let (processor, instrument): (Box<dyn AudioProcessor>, bool) = if self.vst3_requested {
            let product = self
                .vst3_products
                .get(self.vst3_selected)
                .ok_or("select a VST3 class")?;
            let spec = ProcessSpec {
                max_block_frames: 256,
                ..plan.spec()
            };
            // SAFETY: Start explicitly loads the selected trusted VST3 bundle/class.
            let mut owner =
                unsafe { soniclaw::vst3::Instance::load(&product.path, Some(&product.id), spec) }
                    .map_err(|e| e.to_string())?;
            if let Some(state) = &self.vst3_state {
                owner.restore_state(state).map_err(|e| e.to_string())?;
            }
            self.vst3_parameters = owner.parameters().map_err(|e| e.to_string())?;
            let instrument = owner
                .info()
                .map_err(|e| e.to_string())?
                .category
                .contains("Instrument");
            let processor: Box<dyn AudioProcessor> = if self.vst3_direct {
                Box::new(owner.start_direct().map_err(|e| e.to_string())?)
            } else {
                Box::new(
                    if self.vst3_musical {
                        owner.start_buffered_musical(soniclaw::vst3::musical::Config {
                            timeline: self
                                .model
                                .history
                                .current()
                                .timeline
                                .clone()
                                .unwrap_or_default(),
                            recording: false,
                        })
                    } else {
                        owner.start_buffered()
                    }
                    .map_err(|e| e.to_string())?,
                )
            };
            self.vst3_live = Some(owner);
            (processor, instrument)
        } else if self.host_requested {
            self.host_live = None;
            // SAFETY: Start explicitly loads the selected trusted session/library paths.
            let mut resolved = if let Some(session) = &self.host_session {
                Some(
                    unsafe { session.resolve(soniclaw::host::session::MissingPolicy::Placeholder) }
                        .map_err(|e| e.to_string())?,
                )
            } else {
                None
            };
            let instances = if let Some(r) = &mut resolved {
                self.host_mapping = r.slot_indices.clone();
                self.host_missing = r.missing.clone();
                self.host_warnings = r.warnings.clone();
                r.take_instances()
            } else {
                self.host_mapping = (0..self.host_chain.len()).collect();
                self.host_missing.clear();
                self.host_warnings.clear();
                unsafe { soniclaw::host::live::instantiate(&self.host_chain) }
                    .map_err(|e| e.to_string())?
            };
            let (mut rack, processor): (soniclaw::host::live::LiveRack, Box<dyn AudioProcessor>) =
                if self.host_direct {
                    let (r, p) =
                        soniclaw::host::live::LiveRack::start_direct(instances, plan.spec())
                            .map_err(|e| e.to_string())?;
                    (r, Box::new(p))
                } else {
                    let (r, p) = soniclaw::host::live::LiveRack::start_buffered(
                        instances,
                        plan.sample_rate as f64,
                        self.host_buffer_blocks,
                    )
                    .map_err(|e| e.to_string())?;
                    (r, Box::new(p))
                };
            if let Some(r) = &resolved {
                r.configure(&mut rack).map_err(|e| e.to_string())?;
            }
            if let Some(session) = &self.host_session {
                self.host_warnings
                    .extend(rack.restore_editors(session, &self.host_mapping));
            }
            self.host_parameters = self
                .host_mapping
                .iter()
                .position(|i| *i == self.host_node)
                .and_then(|n| rack.parameters(n))
                .unwrap_or(&[])
                .to_vec();
            self.host_live = Some(rack);
            (processor, false)
        } else if let Some(audio) = &self.audition {
            (
                Box::new(soniclaw::catalog::PreviewPlayer::new(audio.clone())?),
                true,
            )
        } else {
            let plugin = self.model.build()?;
            let instrument = plugin.descriptor.is_instrument;
            (
                Box::new(soniclaw::project::ProjectProcessor::new(plugin)),
                instrument,
            )
        };
        let (tx, rx) = rtrb::RingBuffer::new(8);
        let mut engine = Engine::default();
        let (control_sender, control_receiver) = mpsc::sync_channel(1);
        self.live_stats = Arc::new(Default::default());
        engine.push(LiveProcessor {
            rack: None,
            pending: Some(processor),
            control_sender: Some(control_sender),
            stats: self.live_stats.clone(),
            sender: tx,
            sr: plan.sample_rate as f64,
            position: 0,
            test_signal: !instrument && plan.duplex.is_none(),
            tempo: self.tempo.clone(),
        });
        let (mut sender, receiver) = event_channel(128, telemetry);
        if instrument && self.audition.is_none() {
            for (i, note) in [60, 64, 67].into_iter().enumerate() {
                sender.send(MidiMessage::NoteOnId {
                    channel: 1,
                    note,
                    velocity: 0.65,
                    note_id: i as i32 + 1,
                });
            }
        }
        self.sender = Some(sender);
        self.receiver = Some(rx);
        self.pending_control = Some(control_receiver);
        Ok(soniclaw::device::OutputEngine {
            output_bus: None,

            input_bus: None,
            engine,
            receivers: vec![receiver],
        })
    }
    fn finish_device_attempt(&mut self, manager: &soniclaw::device::DeviceManager) {
        self.device_catalog = manager.inventory().clone();
        if let Some(p) = &manager.status().active {
            self.rate = p.sample_rate as f64;
            self.graph_control = self
                .pending_control
                .take()
                .and_then(|rx| rx.try_recv().ok());
            self.status = format!(
                "Playing: {} · {} Hz · {} channels",
                p.output.name, p.sample_rate, p.channels
            );
        } else {
            self.sender = None;
            self.receiver = None;
            self.pending_control = None;
            self.graph_control = None;
            self.status = format!(
                "Audio {:?}: {}",
                manager.status().state,
                manager.status().last_error.as_deref().unwrap_or("stopped")
            );
        }
    }
    fn play(&mut self) -> Result<(), String> {
        let host_requested = self.host_requested;
        let vst3_requested = self.vst3_requested;
        self.stop();
        self.host_requested = host_requested;
        self.vst3_requested = vst3_requested;
        let mut manager = self.device_manager.take().unwrap_or_default();
        let result = manager
            .start(self.device_clock.elapsed(), &mut |p, t| {
                self.make_output_engine(p, t)
            })
            .map_err(|e| e.to_string());
        self.finish_device_attempt(&manager);
        self.device_manager = Some(manager);
        result
    }
    fn poll_devices(&mut self) {
        let vst_error = self
            .vst3_live
            .as_mut()
            .and_then(|p| p.poll().err())
            .map(|e| e.to_string());
        if let Some(error) = vst_error {
            self.stop();
            self.status = error;
        }

        if let Some(rack) = &mut self.host_live {
            rack.poll();
            for event in rack.drain_editor_events() {
                if let soniclaw::host::editors::EditorEvent::Error { message, .. } = event {
                    self.status = message;
                }
            }
            for e in rack.drain_events() {
                if self.host_mapping.get(e.node) == Some(&self.host_node) {
                    if let soniclaw::host::OutputEvent::Parameter(value) = e.event {
                        if let Some(p) = self.host_parameters.iter_mut().find(|p| p.id == value.id)
                        {
                            p.value = value.value;
                        }
                    }
                }
            }
            if rack.stopped() {
                self.stop();
                self.status="Hosted rack stopped; inspect diagnostics and rebuild after restart/rescan requests".into();
            }
        }

        let Some(mut manager) = self.device_manager.take() else {
            return;
        };
        if manager.poll(self.device_clock.elapsed(), &mut |p, t| {
            self.make_output_engine(p, t)
        }) {
            self.finish_device_attempt(&manager);
        }
        self.device_manager = Some(manager);
    }
    fn audio_settings(&mut self, ui: &mut egui::Ui) {
        let Some(manager) = self.device_manager.as_ref() else {
            return;
        };
        if manager
            .status()
            .active
            .as_ref()
            .is_some_and(|p| p.duplex.is_some())
        {
            let mut monitor = manager.monitor_enabled();
            if ui
                .checkbox(&mut monitor, "Monitor through selected output")
                .changed()
            {
                if let Err(e) = manager.set_monitor(monitor) {
                    self.status = e.to_string();
                }
            }
        }
        let action = self.device_panel.show(
            ui,
            &mut self.device_draft,
            &self.device_catalog,
            manager.status(),
        );
        if let Some(action) = action {
            use soniclaw::device::{settings::SettingsAction as A, OutputBackend};
            let result = (|| -> Result<(), soniclaw::device::DeviceError> {
                match action {
                    A::Refresh => {
                        self.device_catalog =
                            soniclaw::device::NativeOutputBackend.enumerate(&self.device_draft)?
                    }
                    A::Save => {
                        self.device_draft.save(&self.device_settings_path)?;
                        self.status = "Audio preferences saved".into();
                    }
                    A::Load => {
                        self.device_draft =
                            soniclaw::device::DevicePreferences::load(&self.device_settings_path)?;
                        self.status = "Preferences loaded; Apply to use them".into();
                    }
                    A::Retry => {
                        self.play()
                            .map_err(|e| -> soniclaw::device::DeviceError { e.into() })?;
                    }
                    A::Stop => self.stop(),
                    A::Apply => {
                        let mut m = self.device_manager.take().unwrap_or_default();
                        let r = m.apply(
                            self.device_draft.clone(),
                            self.device_clock.elapsed(),
                            &mut |p, t| self.make_output_engine(p, t),
                        );
                        self.finish_device_attempt(&m);
                        self.device_manager = Some(m);
                        r?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                self.status = e.to_string();
            }
        }
        ui.separator();
        ui.label("Unified session recording");
        ui.text_edit_singleline(&mut self.recording_path);
        ui.checkbox(
            &mut self.record_dry,
            "Record dry input (otherwise processed output)",
        );
        ui.horizontal(|ui| {
            if ui.button("Start new take").clicked() {
                if let Some(m) = &mut self.device_manager {
                    self.status = match m.record(
                        Path::new(&self.recording_path),
                        if self.record_dry {
                            soniclaw::record::RecordSource::Dry
                        } else {
                            soniclaw::record::RecordSource::Processed
                        },
                    ) {
                        Ok(()) => "Recording started".into(),
                        Err(e) => e.to_string(),
                    };
                }
            }
            if ui.button("Finalize take").clicked() {
                if let Some(m) = &mut self.device_manager {
                    self.status = match m.stop_recording() {
                        Ok(()) => "Recording finalized".into(),
                        Err(e) => e.to_string(),
                    };
                }
            }
        });
    }
    fn update_analysis(&mut self) {
        if let Some(worker) = &self.worker {
            match worker.try_recv() {
                Ok(result) => {
                    self.worker = None;
                    match result {
                        Ok(p) => {
                            if !self.device_running() {
                                self.cpu = p.cpu_percent;
                                self.latency = p.latency;
                                self.rate = 48000.0;
                                self.wave = p.waveform.clone();
                                self.spectrum = p.spectrum.clone();
                                self.peak = p.audio.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
                            }
                            self.preview = Some(p);
                            self.status =
                                "Preview ready · analysis shows actual rendered audio".into();
                        }
                        Err(e) => self.status = e,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.status = "Preview worker disconnected".into();
                }
                Err(_) => {}
            }
        }
        if self.preview_pending && self.worker.is_none() {
            self.preview_pending = false;
            self.preview();
        }
        if let Some(receiver) = &mut self.receiver {
            let mut newest = None;
            for _ in 0..8 {
                if let Ok(packet) = receiver.pop() {
                    for f in packet.samples[..packet.frames * 2].as_chunks::<2>().0 {
                        self.analysis_ring[self.analysis_cursor] = (f[0] + f[1]) * 0.5;
                        self.analysis_cursor = (self.analysis_cursor + 1) % 1024;
                    }
                    newest = Some(packet);
                } else {
                    break;
                }
            }
            if let Some(p) = newest {
                self.cpu = self.cpu * 0.8 + p.cpu * 0.2;
                self.latency = p.latency;
                self.automation_editor.cursor = self
                    .model
                    .history
                    .current()
                    .timeline
                    .as_ref()
                    .and_then(|timeline| {
                        let mut clock = soniclaw::transport::Clock::new(timeline, p.rate).ok()?;
                        clock.seek(p.position).ok()?;
                        Some(clock.frame().ok()?.position.beat)
                    })
                    .unwrap_or(p.position as f64 / p.rate * self.tempo.get() as f64 / 60.0);
                self.rate = p.rate;
                self.wave = p.samples[..p.frames * 2]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|f| (f[0] + f[1]) * 0.5)
                    .collect();
                self.peak = p.samples[..p.frames * 2]
                    .iter()
                    .fold(0.0_f32, |a, x| a.max(x.abs()));
                let mut mono = [0.0; 1024];
                for (i, x) in mono.iter_mut().enumerate() {
                    *x = self.analysis_ring[(self.analysis_cursor + i) % 1024];
                }
                if let Ok(bins) = self.analyzer.analyze(&mono) {
                    self.spectrum = bins.to_vec();
                }
            }
        }
    }
    fn action(&mut self, action: Action) {
        if matches!(
            action,
            Action::Preset(_)
                | Action::Undo
                | Action::Redo
                | Action::Move(..)
                | Action::Remove(_)
                | Action::Add(_)
                | Action::AddCustom(_)
                | Action::Routes
                | Action::Asset
                | Action::GraphApply
                | Action::SamplesApply
                | Action::MidiApply
                | Action::VoiceApply
                | Action::Play
        ) {
            self.loader.cancel();
            self.loading = false;
        }
        let result: Result<(), Box<dyn std::error::Error>> = (|| {
            match action {
                Action::GraphApply
                | Action::SamplesApply
                | Action::MidiApply
                | Action::VoiceApply
                | Action::Audit
                | Action::HostScan
                | Action::HostInspect
                | Action::HostRender => self.panel_action(action)?,
                Action::Stop => {
                    self.stop();
                    self.status = "Stopped · controls and presets remain available".into();
                }
                Action::Play => {
                    self.audition = None;
                    self.play()
                }
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?,
                Action::Preview => self.preview(),
                Action::Preset(i) => {
                    self.stop();
                    self.model.apply(preset(i), "Load preset")?;
                    self.preset_index = i;
                    self.selected = 0;
                    self.route_draft = self.model.snapshot().modulation.unwrap_or_default();
                    self.preview();
                }
                Action::Undo => {
                    self.stop();
                    self.model.undo()?;
                    self.selected = 0;
                    self.route_draft = self.model.snapshot().modulation.unwrap_or_default();
                    self.preview();
                }
                Action::Redo => {
                    self.stop();
                    self.model.redo()?;
                    self.selected = 0;
                    self.route_draft = self.model.snapshot().modulation.unwrap_or_default();
                    self.preview();
                }
                Action::Save => {
                    self.model.save(Path::new(&self.path))?;
                    self.status = "Preset saved".into();
                }
                Action::Load => {
                    self.stop();
                    self.loader.submit(
                        soniclaw::project_loading::LoadRequest::File(self.path.clone().into()),
                        None,
                    )?;
                    self.loading = true;
                    self.status = "Loading project on a worker…".into();
                }
                Action::ImportBundle => {
                    self.stop();
                    self.loader.submit(
                        soniclaw::project_loading::LoadRequest::Bundle {
                            path: self.single_bundle.clone().into(),
                            destination: self.import_directory.clone().into(),
                        },
                        None,
                    )?;
                    self.loading = true;
                    self.status = "Verifying and importing bundle on a worker…".into();
                }
                Action::Recover => {
                    self.stop();
                    self.loader.submit(
                        soniclaw::project_loading::LoadRequest::Recovery {
                            directory: self.recovery_directory.clone().into(),
                            product: soniclaw_workbench::APP_ID.into(),
                        },
                        None,
                    )?;
                    self.loading = true;
                    self.status = "Checking recovery snapshots on a worker…".into();
                }
                Action::Checkpoint => {
                    self.checkpoint()?;
                }
                Action::SingleBundle => {
                    if self.storage_worker.is_some() {
                        return Err("bundle export already running".into());
                    }
                    let state = self.model.snapshot();
                    let root = self.model.root.clone();
                    let path = PathBuf::from(&self.single_bundle);
                    let (tx, rx) = mpsc::sync_channel(1);
                    std::thread::spawn(move || {
                        let result = soniclaw::project_storage::pack_bundle(&state, &root, &path)
                            .map(|_| format!("Saved {}", path.display()))
                            .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    });
                    self.storage_worker = Some(rx);
                    self.status = "Packing project and WAV assets on a worker…".into();
                }
                Action::Bundle => {
                    self.model.export_bundle(Path::new(&self.bundle))?;
                    self.status = "Portable project exported with WAV assets".into();
                }
                Action::Move(index, delta) => {
                    self.stop();
                    let mut nodes = self.model.snapshot().nodes;
                    let to = (index as isize + delta) as usize;
                    if to < nodes.len() {
                        nodes.swap(index, to);
                        self.model.edit_chain(nodes, "Move processor")?;
                        self.selected = to;
                        self.preview();
                    }
                }
                Action::Remove(index) => {
                    self.stop();
                    let mut nodes = self.model.snapshot().nodes;
                    if nodes.len() > 1 {
                        nodes.remove(index);
                        self.model.edit_chain(nodes, "Remove processor")?;
                        self.selected = self
                            .selected
                            .min(self.model.history.current().nodes.len() - 1);
                        self.preview();
                    }
                }
                Action::AddCustom(processor_id) => {
                    self.stop();
                    let mut nodes = self.model.snapshot().nodes;
                    let id = (1..=256)
                        .map(|i| format!("custom{i}"))
                        .find(|id| !nodes.iter().any(|n| &n.id == id))
                        .ok_or("custom node limit")?;
                    nodes.push(soniclaw_workbench::registry().node(id, processor_id)?);
                    self.model.edit_chain(nodes, "Add registered processor")?;
                    self.selected = self.model.snapshot().nodes.len() - 1;
                    self.preview();
                }
                Action::Add(kind) => {
                    self.stop();
                    let mut nodes = self.model.snapshot().nodes;
                    if nodes.len() >= 16 {
                        return Err("workbench racks are limited to 16 nodes".into());
                    }
                    let mut number = 1;
                    while nodes.iter().any(|n| n.id == format!("fx{number}")) {
                        number += 1;
                    }
                    nodes.push(RackNode::new(format!("fx{number}"), kind));
                    self.model.edit_chain(nodes, "Add processor")?;
                    self.selected = self.model.history.current().nodes.len() - 1;
                    self.preview();
                }
                Action::Routes => {
                    self.stop();
                    let mut state = self.model.snapshot();
                    state.modulation = if self.route_draft.sources.is_empty() {
                        None
                    } else {
                        Some(self.route_draft.clone())
                    };
                    self.model.apply(state, "Edit modulation")?;
                    self.preview();
                }
                Action::Asset => {
                    self.stop();
                    let mut state = self.model.snapshot();
                    let node = state
                        .nodes
                        .get_mut(self.selected)
                        .ok_or("select a resource node")?;
                    if !matches!(
                        node.kind,
                        ProcessorKind::Granular | ProcessorKind::Convolution
                    ) {
                        return Err("select a Granular or Convolution node to load its WAV".into());
                    }
                    let mut path = PathBuf::from(&self.asset_path);
                    if path.is_absolute() {
                        let file = soniclaw::read_audio_with_limit(&path, 128 * 1024 * 1024)?;
                        if file.channels > 2 {
                            return Err("use a mono/stereo WAV".into());
                        }
                        let folder = self.model.root.join("assets");
                        std::fs::create_dir_all(&folder)?;
                        let mut i = 1;
                        while folder.join(format!("import-{i}.wav")).exists() {
                            i += 1;
                        }
                        let name = format!("assets/import-{i}.wav");
                        std::fs::copy(&path, self.model.root.join(&name))?;
                        path = PathBuf::from(name);
                    }
                    let id = format!("{}-asset", node.id);
                    node.asset = Some(id.clone());
                    state.assets.retain(|a| a.id != id);
                    state.assets.push(Asset {
                        id,
                        path: path.to_string_lossy().into_owned(),
                        root_note: 57,
                    });
                    self.model.apply(state, "Load WAV asset")?;
                    self.preview();
                }
            }
            Ok(())
        })();
        if matches!(
            action,
            Action::Preset(_)
                | Action::Load
                | Action::Undo
                | Action::Redo
                | Action::Move(..)
                | Action::Remove(_)
                | Action::Add(_)
                | Action::AddCustom(_)
        ) {
            self.graph_draft = None;
            self.sample_draft = None;
            self.voice_draft = None;
            self.sample_cache.clear();
            self.midi_draft = self.model.snapshot().midi.unwrap_or_default();
        }
        if let Err(error) = result {
            self.status = error.to_string();
        }
    }
    fn checkpoint(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.autosave_worker.is_none() {
            self.autosave_worker = Some(soniclaw::project_loading::AutosaveWorker::new(
                soniclaw::project_storage::RecoveryStore::new(
                    PathBuf::from(&self.recovery_directory),
                    5,
                )?,
            ));
        }
        self.autosave_worker
            .as_ref()
            .unwrap()
            .try_checkpoint(self.model.snapshot(), self.model.root.clone())?;
        self.autosave_at = Instant::now();
        Ok(())
    }
    fn poll_project_work(&mut self) {
        if let Some(outcome) = self.loader.try_poll() {
            self.loading = false;
            let result = outcome
                .result
                .and_then(|loaded| self.model.install_loaded(loaded).map_err(|e| e.to_string()));
            match result {
                Ok(()) => {
                    self.selected = 0;
                    self.graph_draft = None;
                    self.sample_draft = None;
                    self.voice_draft = None;
                    self.sample_cache.clear();
                    self.route_draft = self.model.snapshot().modulation.unwrap_or_default();
                    self.midi_draft = self.model.snapshot().midi.unwrap_or_default();
                    self.path = self.model.root.join("project.json").display().to_string();
                    self.preview();
                }
                Err(e) => self.status = format!("Load failed: {e}"),
            }
        }
        if let Some(rx) = &self.storage_worker {
            match rx.try_recv() {
                Ok(result) => {
                    self.status = result.unwrap_or_else(|e| e);
                    self.storage_worker = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Bundle worker stopped".into();
                    self.storage_worker = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(worker) = &self.autosave_worker {
            if let Some(result) = worker.try_poll() {
                self.status = match result {
                    Ok(_) => "Recovery checkpoint saved".into(),
                    Err(e) => format!("Recovery checkpoint failed: {e}"),
                };
            }
        }
        if self.autosave && !self.loading && self.autosave_at.elapsed() >= Duration::from_secs(30) {
            if let Err(e) = self.checkpoint() {
                self.status = e.to_string();
                self.autosave_at = Instant::now();
            }
        }
    }
    fn browser_panel(&mut self, ui: &mut egui::Ui) {
        use soniclaw::catalog::{browser::BrowserAction as A, PresetCatalog};
        ui.heading("Preset and asset browser");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.browser_root);
            if ui.button("Open catalog").clicked() {
                match PresetCatalog::open(PathBuf::from(&self.browser_root)) {
                    Ok(c) => {
                        self.catalog = Some(c);
                        self.browser.selected = None;
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        });
        let Some(catalog) = &mut self.catalog else {
            ui.label(
                "Open a dedicated writable folder to store presets and their searchable metadata.",
            );
            return;
        };
        let action = self.browser.show(ui, catalog);
        if let Some(id) = &self.browser.selected {
            if let Ok(entry) = catalog.entry(id) {
                if self.browser_edit_id != *id {
                    self.browser_edit_id = id.clone();
                    self.browser_name = entry.name.clone();
                    self.browser_tags = entry.tags.join(", ");
                    self.browser_bank = entry.bank.clone();
                    self.browser_asset_root = entry.asset_root.display().to_string();
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.browser_name);
                ui.label("Bank");
                ui.text_edit_singleline(&mut self.browser_bank);
            });
            ui.horizontal(|ui| {
                ui.label("Tags (comma separated)");
                ui.text_edit_singleline(&mut self.browser_tags);
                if ui.button("Save metadata").clicked() {
                    let r = (|| -> Result<(), Box<dyn std::error::Error>> {
                        let favourite = catalog.entry(id)?.favourite;
                        catalog.edit_metadata(
                            id,
                            self.browser_name.clone(),
                            self.browser_tags
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .collect(),
                            self.browser_bank.clone(),
                            favourite,
                        )
                    })();
                    if let Err(e) = r {
                        self.status = e.to_string();
                    }
                }
            });
            if ui.button("Check asset references").clicked() {
                self.browser_missing = match catalog.missing_assets(id) {
                    Ok(m) => m
                        .into_iter()
                        .map(|(a, p)| format!("{a}: {}", p.display()))
                        .collect(),
                    Err(e) => vec![e.to_string()],
                };
            }
            for missing in &self.browser_missing {
                ui.label(missing);
            }
            ui.label("Relocate assets: existing relative paths are retained unless one replacement is specified.");
            ui.text_edit_singleline(&mut self.browser_asset_root);
            ui.horizontal(|ui| {
                ui.label("Asset ID");
                ui.text_edit_singleline(&mut self.browser_asset_id);
                ui.label("New relative audio path (WAV / AIFF / FLAC)");
                ui.text_edit_singleline(&mut self.browser_asset_path);
            });
            if ui.button("Validate and relocate").clicked() {
                let mut paths = std::collections::BTreeMap::new();
                if !self.browser_asset_id.is_empty() {
                    paths.insert(
                        self.browser_asset_id.clone(),
                        self.browser_asset_path.clone(),
                    );
                }
                self.status =
                    match catalog.relocate(id, Path::new(&self.browser_asset_root), &paths) {
                        Ok(()) => "Asset references relocated".into(),
                        Err(e) => e.to_string(),
                    };
            }
            ui.horizontal(|ui| {
                ui.label("Export selected .soniclaw");
                ui.text_edit_singleline(&mut self.single_bundle);
                if ui.button("Export selected").clicked() {
                    self.status = match catalog.export_preset(id, Path::new(&self.single_bundle)) {
                        Ok(()) => "Preset bundle exported".into(),
                        Err(e) => e.to_string(),
                    };
                }
            });
        }
        if let Some(action) = action {
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                match action {
                    A::Store => {
                        let id = self.catalog.as_mut().unwrap().insert(
                            &self.model.snapshot(),
                            &self.model.root,
                            vec![],
                            "User".into(),
                        )?;
                        self.browser.selected = Some(id);
                    }
                    A::ToggleFavourite(id) => {
                        let c = self.catalog.as_mut().unwrap();
                        let e = c.entry(&id)?.clone();
                        c.edit_metadata(&id, e.name, e.tags, e.bank, !e.favourite)?;
                    }
                    A::Remove(id) => {
                        self.catalog.as_mut().unwrap().remove(&id)?;
                        self.browser.selected = None;
                    }
                    A::Load(id) => {
                        let (s, root) = self.catalog.as_ref().unwrap().project(&id)?;
                        self.stop();
                        self.audition = None;
                        self.loader.submit(
                            soniclaw::project_loading::LoadRequest::Snapshot {
                                state: Box::new(s),
                                root,
                            },
                            None,
                        )?;
                        self.loading = true;
                    }
                    A::Preview(id) => {
                        if self.worker.is_some() {
                            return Err("preview already running; retry when ready".into());
                        }
                        self.browser_preview_id = Some(id.clone());
                        self.preview = None;
                        let (s, root) = self.catalog.as_ref().unwrap().project(&id)?;
                        let (tx, rx) = mpsc::sync_channel(1);
                        std::thread::spawn(move || {
                            let _ = tx.send(render(s, root, 2.0));
                        });
                        self.worker = Some(rx);
                        self.status = "Rendering selected preset on a worker".into();
                    }
                    A::Audition => {
                        if self.worker.is_some() || self.browser_preview_id != self.browser.selected
                        {
                            return Err("render the selected preset before auditioning".into());
                        }
                        let p = self.preview.as_ref().ok_or("render a preview first")?;
                        self.audition = Some(Arc::new(soniclaw::io::AudioFile {
                            sample_rate: 48000,
                            channels: 2,
                            samples: p.audio.clone(),
                        }));
                        self.play()
                            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                self.status = e.to_string();
            }
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        let node = self
            .model
            .history
            .current()
            .nodes
            .get(self.selected)
            .cloned();
        let Some(node) = node else {
            return;
        };
        ui.horizontal(|ui| {
            ui.heading(&node.id);
            ui.label(egui::RichText::new(format!("{:?}", node.kind)).color(ACCENT));
        });
        ui.label("Base values are saved; the green readout includes live modulation.");
        let parameters: Vec<_> = self
            .model
            .parameters
            .iter()
            .filter(|p| p.info().id.starts_with(&format!("{}.", node.id)))
            .cloned()
            .collect();
        if let Some(custom) = &node.custom {
            if let Some(commit) =
                soniclaw_workbench::registry().show_editor(&custom.processor_id, ui, &parameters)
            {
                if commit {
                    if let Err(e) = self.model.record_controls("Adjust registered parameter") {
                        self.status = e.to_string();
                    }
                }
                return;
            }
        }
        let mut commit = false;
        egui::ScrollArea::vertical()
            .id_salt("controls")
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("parameter-grid")
                    .num_columns(3)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        let width = (ui.available_width() - 48.0) / 3.0;
                        for (i, p) in parameters.iter().enumerate() {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgb(24, 33, 45))
                                .corner_radius(9)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.set_min_width(width.max(120.0));
                                    let info = p.info();
                                    ui.label(egui::RichText::new(info.name).strong());
                                    let mut v = p.get();
                                    let response = ui.add(
                                        egui::Slider::new(&mut v, info.min..=info.max)
                                            .logarithmic(
                                                info.min > 0.0 && info.max / info.min > 100.0,
                                            )
                                            .suffix(format!(" {}", info.unit)),
                                    );
                                    if response.changed() {
                                        let _ = p.set(v);
                                    }
                                    if response.double_clicked() {
                                        let _ = p.set(info.default);
                                        commit = true;
                                    }
                                    if response.drag_stopped()
                                        || (response.changed() && !response.dragged())
                                    {
                                        commit = true;
                                    }
                                    ui.label(
                                        egui::RichText::new(format!("live  {:.3}", p.modulated()))
                                            .small()
                                            .color(ACCENT),
                                    );
                                });
                            if i % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
            });
        if commit {
            if let Err(e) = self.model.record_controls("Adjust parameter") {
                self.status = e.to_string();
            }
        }
    }
    fn routes(&mut self, ui: &mut egui::Ui, action: &mut Option<Action>) {
        ui.heading("Modulation matrix");
        ui.label("Routes are normalized offsets; applying changes stops and rebuilds the rack.");
        ui.horizontal(|ui| {
            if ui.button("+ LFO").clicked() && self.route_draft.sources.len() < 16 {
                self.route_draft.sources.push(SourceConfig::Lfo {
                    shape: LfoShape::Sine,
                    rate_hz: 0.5,
                    beats: None,
                });
            }
            if ui.button("+ Envelope").clicked() && self.route_draft.sources.len() < 16 {
                self.route_draft.sources.push(SourceConfig::Envelope {
                    stages: vec![
                        EnvelopeStage {
                            seconds: 0.1,
                            level: 1.0,
                            curve: 0.0,
                        },
                        EnvelopeStage {
                            seconds: 0.3,
                            level: 0.6,
                            curve: 0.0,
                        },
                        EnvelopeStage {
                            seconds: 0.4,
                            level: 0.0,
                            curve: 0.0,
                        },
                    ],
                    sustain_stage: 1,
                });
            }
            if ui.button("+ Macro").clicked()
                && self.route_draft.macros.len() < 8
                && self.route_draft.sources.len() < 16
            {
                let index = self.route_draft.macros.len();
                self.route_draft.macros.push(0.5);
                self.route_draft.sources.push(SourceConfig::Macro { index });
            }
            if ui.button("Apply routing").clicked() {
                *action = Some(Action::Routes);
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("route-scroll")
            .max_height(280.0)
            .show(ui, |ui| {
                for (i, source) in self.route_draft.sources.iter_mut().enumerate() {
                    egui::CollapsingHeader::new(format!("Source {}", i + 1))
                        .default_open(true)
                        .show(ui, |ui| match source {
                            SourceConfig::Lfo {
                                shape,
                                rate_hz,
                                beats,
                            } => {
                                ui.horizontal(|ui| {
                                    egui::ComboBox::from_id_salt(("shape", i))
                                        .selected_text(format!("{shape:?}"))
                                        .show_ui(ui, |ui| {
                                            for s in [
                                                LfoShape::Sine,
                                                LfoShape::Triangle,
                                                LfoShape::Saw,
                                                LfoShape::Square,
                                                LfoShape::SampleHold,
                                            ] {
                                                ui.selectable_value(shape, s, format!("{s:?}"));
                                            }
                                        });
                                    ui.add(
                                        egui::DragValue::new(rate_hz)
                                            .range(0.01..=40.0)
                                            .speed(0.05)
                                            .suffix(" Hz"),
                                    );
                                    let mut sync = beats.is_some();
                                    if ui.checkbox(&mut sync, "Beat sync").changed() {
                                        *beats = if sync { Some(4.0) } else { None };
                                    }
                                    if let Some(b) = beats {
                                        ui.add(
                                            egui::DragValue::new(b)
                                                .range(0.0625..=16.0)
                                                .speed(0.25)
                                                .suffix(" beats"),
                                        );
                                    }
                                });
                                let time = ui.input(|i| i.time) as f32;
                                let hz = beats.map_or(*rate_hz, |b| self.tempo.get() / 60.0 / b);
                                let points: Vec<_> = (0..160)
                                    .map(|x| {
                                        let p = x as f32 / 159.0;
                                        let phase = (time * hz + p * 2.0).fract();
                                        match shape {
                                            LfoShape::Sine => (std::f32::consts::TAU * phase).sin(),
                                            LfoShape::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                                            LfoShape::Saw => phase * 2.0 - 1.0,
                                            LfoShape::Square => {
                                                if phase < 0.5 {
                                                    1.0
                                                } else {
                                                    -1.0
                                                }
                                            }
                                            LfoShape::SampleHold => 0.0,
                                        }
                                    })
                                    .collect();
                                plot(ui, &points, 55.0, false, self.rate);
                            }
                            SourceConfig::Envelope {
                                stages,
                                sustain_stage,
                            } => {
                                for (j, s) in stages.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("Stage {}", j + 1));
                                        ui.add(
                                            egui::DragValue::new(&mut s.seconds)
                                                .range(0.001..=30.0)
                                                .speed(0.01)
                                                .suffix(" s"),
                                        );
                                        ui.add(
                                            egui::Slider::new(&mut s.level, 0.0..=1.0)
                                                .text("level"),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut s.curve)
                                                .range(-4.0..=4.0)
                                                .speed(0.1)
                                                .prefix("curve "),
                                        );
                                    });
                                }
                                ui.add(
                                    egui::Slider::new(sustain_stage, 0..=stages.len() - 2)
                                        .text("Sustain stage (zero based)"),
                                );
                            }
                            SourceConfig::Macro { index } => {
                                if let Some(p) = self
                                    .model
                                    .parameters
                                    .iter()
                                    .find(|p| p.info().id == format!("mod.macro{}", *index + 1))
                                {
                                    let mut value = p.get();
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut value, 0.0..=1.0)
                                                .text(format!("Macro {}", *index + 1)),
                                        )
                                        .changed()
                                    {
                                        let _ = p.set(value);
                                    }
                                } else {
                                    ui.label("Apply routing to create the live macro control.");
                                }
                            }
                            SourceConfig::MidiCc {
                                channel,
                                controller,
                            } => {
                                ui.label(format!("MIDI CC {controller}, channel {channel}"));
                            }
                            SourceConfig::Pressure { channel } => {
                                ui.label(format!("Pressure, channel {channel}"));
                            }
                        });
                }
                let destinations: Vec<_> = self
                    .model
                    .parameters
                    .iter()
                    .filter(|p| !p.info().id.starts_with("mod."))
                    .map(|p| p.info().id.to_owned())
                    .collect();
                for (i, route) in self.route_draft.routes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label("Source");
                        ui.add(
                            egui::DragValue::new(&mut route.source)
                                .range(0..=self.route_draft.sources.len().saturating_sub(1)),
                        );
                        egui::ComboBox::from_id_salt(("dest", i))
                            .selected_text(&route.destination)
                            .show_ui(ui, |ui| {
                                for d in &destinations {
                                    ui.selectable_value(&mut route.destination, d.clone(), d);
                                }
                            });
                        ui.add(egui::Slider::new(&mut route.amount, -1.0..=1.0).text("amount"));
                    });
                }
                if ui.button("+ Route").clicked()
                    && !self.route_draft.sources.is_empty()
                    && self.route_draft.routes.len() < 64
                {
                    if let Some(destination) = destinations.first() {
                        self.route_draft.routes.push(ModulationRoute {
                            source: 0,
                            destination: destination.clone(),
                            amount: 0.1,
                        });
                    }
                }
            });
    }
    fn assets(&mut self, ui: &mut egui::Ui, action: &mut Option<Action>) {
        ui.heading("Sample and impulse assets");
        ui.label("Select a Granular or Convolution node, then load a project-relative WAV. An absolute path is imported into the project assets folder.");
        ui.text_edit_singleline(&mut self.asset_path);
        if ui.button("Load / import WAV (stops audio)").clicked() {
            *action = Some(Action::Asset);
        }
        for asset in &self.model.history.current().assets {
            ui.label(format!("{}  ·  {}", asset.id, asset.path));
        }
        ui.separator();
        ui.label(format!("Project root: {}", self.model.root.display()));
        ui.label("IR engines are prepared off the audio thread. Rack changes rebuild while stopped. The reusable convolution_pair API supports live crossfades with deferred retirement.");
    }
}
fn plot(ui: &mut egui::Ui, values: &[f32], height: f32, spectrum: bool, rate: f64) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 8.0, egui::Color32::from_rgb(8, 14, 22));
    for y in 1..4 {
        let at = rect.top() + height * y as f32 / 4.0;
        p.line_segment(
            [egui::pos2(rect.left(), at), egui::pos2(rect.right(), at)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(28, 40, 53)),
        );
    }
    let points: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = if spectrum {
                let hz = (i as f32 * rate as f32 / 1024.0).max(20.0);
                (hz / 20.0).ln() / (24000.0_f32 / 20.0).ln()
            } else {
                i as f32 / values.len().saturating_sub(1).max(1) as f32
            };
            let y = if spectrum {
                (20.0 * v.abs().max(1e-6).log10() + 72.0) / 72.0
            } else {
                0.5 + v.clamp(-1.0, 1.0) * 0.45
            };
            egui::pos2(
                rect.left() + rect.width() * x.clamp(0.0, 1.0),
                rect.bottom() - height * y.clamp(0.0, 1.0),
            )
        })
        .collect();
    if points.len() > 1 {
        p.add(egui::Shape::line(points, egui::Stroke::new(1.8, ACCENT)));
    }
}
impl eframe::App for Workbench {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.frames += 1;
        self.update_analysis();
        self.poll_panels();
        self.poll_project_work();
        self.poll_devices();
        if let Some(c) = &mut self.graph_control {
            c.collect_retired();
        }
        let ctx = root.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(33));
        let mut action = None;
        egui::Panel::top("header").show(root, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("soniclaw")
                        .size(28.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.label(
                    egui::RichText::new(format!("WORKBENCH / {}", env!("CARGO_PKG_VERSION")))
                        .size(12.0)
                        .color(egui::Color32::GRAY),
                );
                ui.separator();
                if ui.button("▶ Play").clicked() {
                    action = Some(Action::Play);
                }
                if ui.button("■ Stop").clicked() {
                    action = Some(Action::Stop);
                }
                if ui.button("Render preview").clicked() {
                    action = Some(Action::Preview);
                }
                if ui.button("Undo").clicked() {
                    action = Some(Action::Undo);
                }
                if ui.button("Redo").clicked() {
                    action = Some(Action::Redo);
                }
                let mut tempo = self.tempo.get();
                if ui
                    .add(
                        egui::DragValue::new(&mut tempo)
                            .range(20.0..=400.0)
                            .speed(1.0)
                            .suffix(" BPM"),
                    )
                    .changed()
                {
                    let _ = self.tempo.set(tempo);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(if self.device_running() {
                            "● DEVICE LIVE"
                        } else {
                            "● OFFLINE PREVIEW"
                        })
                        .color(ACCENT),
                    );
                });
            });
            ui.add_space(8.0);
        });
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("Rust · prepared DSP · no audio device on startup");
                });
            });
        });
        egui::Panel::left("rack")
            .resizable(false)
            .exact_size(238.0)
            .show(root, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("rack-scroll")
                    .show(ui, |ui| {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new("PRESETS")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        for (i, name) in [
                            "Motion Rack",
                            "Multiband Glue",
                            "Granular Cloud",
                            "Spectral Space",
                            "Poly Motion",
                            "Sampler Studio",
                            "Tape Colour Rack",
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if ui.selectable_label(self.preset_index == i, name).clicked() {
                                action = Some(Action::Preset(i));
                            }
                        }
                        ui.separator();
                        ui.label(
                            egui::RichText::new("SIGNAL CHAIN")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        for (i, node) in self.model.history.current().nodes.iter().enumerate() {
                            egui::Frame::new()
                                .fill(if self.selected == i {
                                    egui::Color32::from_rgb(30, 59, 64)
                                } else {
                                    egui::Color32::from_rgb(22, 30, 42)
                                })
                                .corner_radius(7)
                                .inner_margin(8)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .selectable_label(
                                                self.selected == i,
                                                format!("{:02}  {}", i + 1, node.id),
                                            )
                                            .clicked()
                                        {
                                            self.selected = i;
                                        }
                                        if i > 0 && ui.small_button("Up").clicked() {
                                            action = Some(Action::Move(i, -1));
                                        }
                                        if i + 1 < self.model.history.current().nodes.len()
                                            && ui.small_button("Dn").clicked()
                                        {
                                            action = Some(Action::Move(i, 1));
                                        }
                                        if ui.small_button("x").clicked() {
                                            action = Some(Action::Remove(i));
                                        }
                                    });
                                    ui.label(
                                        egui::RichText::new(format!("{:?}", node.kind))
                                            .small()
                                            .color(egui::Color32::GRAY),
                                    );
                                });
                            ui.add_space(4.0);
                        }
                        egui::ComboBox::from_id_salt("add-fx")
                            .selected_text("+ Add processor")
                            .show_ui(ui, |ui| {
                                for kind in [
                                    ProcessorKind::StateVariable,
                                    ProcessorKind::Ladder,
                                    ProcessorKind::Multiband,
                                    ProcessorKind::DynamicEq,
                                    ProcessorKind::SpectralFreeze,
                                    ProcessorKind::PitchShift,
                                    ProcessorKind::FrequencyShift,
                                    ProcessorKind::Phaser,
                                    ProcessorKind::Flanger,
                                    ProcessorKind::FdnReverb,
                                    ProcessorKind::MultiTapDelay,
                                    ProcessorKind::Gain,
                                    ProcessorKind::StereoWidth,
                                    ProcessorKind::LookaheadLimiter,
                                ] {
                                    if ui.button(format!("{kind:?}")).clicked() {
                                        action = Some(Action::Add(kind));
                                    }
                                }
                            });
                        ui.separator();
                        egui::ComboBox::from_id_salt("registered-processors")
                            .selected_text("+ Registered processor")
                            .show_ui(ui, |ui| {
                                for m in soniclaw_workbench::registry().entries() {
                                    if ui.button(format!("{} / {}", m.category, m.name)).clicked() {
                                        action = Some(Action::AddCustom(m.id));
                                    }
                                }
                            });
                        ui.label("Preset JSON");
                        ui.text_edit_singleline(&mut self.path);
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                action = Some(Action::Save);
                            }
                            if ui.button("Load").clicked() {
                                action = Some(Action::Load);
                            }
                        });
                        ui.add_space(6.0);
                        ui.label("Portable project folder");
                        ui.text_edit_singleline(&mut self.bundle);
                        if ui.button("Export bundle + assets").clicked() {
                            action = Some(Action::Bundle);
                        }
                        ui.label("Single-file project (.soniclaw)");
                        ui.text_edit_singleline(&mut self.single_bundle);
                        if ui.button("Pack project + assets").clicked() {
                            action = Some(Action::SingleBundle);
                        }
                        ui.label("Import into new folder");
                        ui.text_edit_singleline(&mut self.import_directory);
                        if ui.button("Import .soniclaw").clicked() {
                            action = Some(Action::ImportBundle);
                        }
                        if self.loading {
                            ui.label("Loading…");
                        }
                        ui.separator();
                        ui.checkbox(&mut self.autosave, "Autosave every 30 seconds");
                        ui.add_enabled(
                            self.autosave_worker.is_none(),
                            egui::TextEdit::singleline(&mut self.recovery_directory),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Checkpoint").clicked() {
                                action = Some(Action::Checkpoint);
                            }
                            if ui.button("Recover latest").clicked() {
                                action = Some(Action::Recover);
                            }
                        });
                        ui.add_space(10.0);
                        if let Some(preview) = &self.preview {
                            if ui.button("Export preview WAV").clicked() {
                                let path = self.model.root.join("workbench-preview.wav");
                                self.status = match soniclaw::io::write_wav(
                                    &path,
                                    48000,
                                    2,
                                    &preview.audio,
                                ) {
                                    Ok(()) => format!("Exported {}", path.display()),
                                    Err(e) => e.to_string(),
                                };
                            }
                        }
                    });
            });
        egui::CentralPanel::default().show(root, |outer| {
            egui::ScrollArea::vertical()
                .id_salt("workbench-central")
                .show(outer, |ui| {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.heading(&self.model.history.current().name);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{:.1}% render budget  |  {} frames / {:.2} ms",
                                    self.cpu,
                                    self.latency,
                                    self.latency as f64 / self.rate * 1000.0
                                ))
                                .monospace()
                                .color(ACCENT),
                            );
                        });
                    });
                    ui.label(
                "A native playground for modulation, dynamics, grains, spectra and convolution.",
            );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        for (i, name) in [
                            "Controls",
                            "Modulation",
                            "Assets",
                            "Graph",
                            "Sample map",
                            "MIDI",
                            "Voice",
                            "Diagnostics",
                            "Host",
                            "Audio",
                            "Presets",
                            "Automation",
                            "Transport",
                            "VST3",
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            ui.selectable_value(&mut self.tab, i, name);
                        }
                    });
                    ui.separator();
                    match self.tab {
                        1 => self.routes(ui, &mut action),
                        2 => self.assets(ui, &mut action),
                        3 => self.graph_panel(ui, &mut action),
                        4 => self.samples_panel(ui, &mut action),
                        5 => self.midi_panel(ui, &mut action),
                        6 => self.voice_panel(ui, &mut action),
                        7 => self.diagnostics_panel(ui, &mut action),
                        8 => self.host_panel(ui, &mut action),
                        9 => self.audio_settings(ui),
                        10 => self.browser_panel(ui),
                        11 => self.automation_panel(ui),
                        12 => self.transport_panel(ui),
                        13 => self.vst3_panel(ui),
                        _ => self.controls(ui),
                    }
                    ui.add_space(12.0);
                    ui.columns(2, |columns| {
                        columns[0].label("OUTPUT SPECTRUM  /  actual signal");
                        plot(&mut columns[0], &self.spectrum, 140.0, true, self.rate);
                        columns[1].label("WAVEFORM  /  actual signal");
                        plot(&mut columns[1], &self.wave, 140.0, false, self.rate);
                    });
                    ui.add_space(8.0);
                    let filter = self
                        .model
                        .parameters
                        .iter()
                        .find(|p| p.info().id == "tone.frequency")
                        .cloned();
                    if let Some(filter) = filter {
                        ui.label("FILTER RESPONSE  /  linear low-pass approximation");
                        let fc = filter.modulated();
                        let q = self
                            .model
                            .parameters
                            .iter()
                            .find(|p| p.info().id == "tone.q")
                            .map_or(0.707, |p| p.modulated());
                        let curve: Vec<_> = (0..256)
                            .map(|i| {
                                let hz = 20.0 * 1000.0_f32.powf(i as f32 / 255.0);
                                let r = (std::f32::consts::PI * hz.min(self.rate as f32 * 0.45)
                                    / self.rate as f32)
                                    .tan()
                                    / (std::f32::consts::PI * fc.min(self.rate as f32 * 0.45)
                                        / self.rate as f32)
                                        .tan();
                                let gain = 1.0 / ((1.0 - r * r).powi(2) + (r / q).powi(2)).sqrt();
                                (20.0 * gain.max(1e-6).log10() / 60.0 + 1.0).clamp(0.0, 1.0) * 2.0
                                    - 1.0
                            })
                            .collect();
                        plot(ui, &curve, 75.0, false, self.rate);
                    }
                    ui.label(format!(
                        "Peak {:.1} dBFS  ·  {:.0} Hz  ·  Drag controls; double-click to reset",
                        20.0 * self.peak.max(1e-8).log10(),
                        self.rate
                    ));
                    for message in soniclaw::ui::piano_keyboard(ui, 60, 2, &mut self.held) {
                        if let Some(sender) = &mut self.sender {
                            sender.send(message);
                        }
                    }
                });
        });
        if let Some(action) = action {
            self.action(action);
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            if let Some(path) = &self.screenshot {
                let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Err(error) = image::save_buffer(
                    path,
                    &pixels,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    self.status = error.to_string();
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.screenshot.is_some()
            && !self.screenshot_requested
            && self.preview.is_some()
            && self.worker.is_none()
            && self.frames > 10
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }
}
pub fn run(args: &[std::ffi::OsString]) -> Result<(), Box<dyn std::error::Error>> {
    let screenshot = args
        .iter()
        .position(|x| x == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    let option = |name: &str, max: usize| -> Result<usize, Box<dyn std::error::Error>> {
        let value = args
            .iter()
            .position(|x| x == name)
            .map(|i| {
                args.get(i + 1)
                    .ok_or("missing option value")?
                    .to_str()
                    .ok_or("invalid option")?
                    .parse::<usize>()
                    .map_err(|_| "invalid index")
            })
            .transpose()?
            .unwrap_or(0);
        if value > max {
            return Err("index out of range".into());
        }
        Ok(value)
    };
    let mut app = Workbench::new(screenshot, option("--preset", 6)?, option("--tab", 13)?)?;
    if args.iter().any(|a| a == "--unified") {
        app.device_draft.duplex = Some(Default::default());
    }
    if let Some(i) = args.iter().position(|a| a == "--catalog") {
        let path = args.get(i + 1).ok_or("missing catalog directory")?;
        app.browser_root = PathBuf::from(path).display().to_string();
        app.catalog = Some(soniclaw::catalog::PresetCatalog::open(PathBuf::from(path))?);
    }
    eframe::run_native(
        "Soniclaw · DSP Workbench",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1360.0, 960.0])
                .with_min_inner_size([1120.0, 820.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(13, 19, 28);
            visuals.window_fill = egui::Color32::from_rgb(20, 29, 41);
            visuals.selection.bg_fill = egui::Color32::from_rgb(26, 94, 88);
            visuals.hyperlink_color = ACCENT;
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_menu_action_can_save_and_reload_without_audio() {
        let temp =
            std::env::temp_dir().join(format!("soniclaw-custom-editor-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        let mut app = Workbench::new(None, 6, 0).unwrap();
        app.model.root = temp.clone();
        app.action(Action::AddCustom("org.soniclaw.example.tape"));
        assert_eq!(app.model.snapshot().nodes.len(), 2);
        let id = app.model.snapshot().nodes[1].id.clone();
        app.model
            .parameters
            .iter()
            .find(|p| p.info().id == format!("{id}.drive"))
            .unwrap()
            .set(5.0)
            .unwrap();
        app.path = temp.join("project.json").display().to_string();
        app.action(Action::Save);
        app.action(Action::Load);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.loading {
            app.poll_project_work();
            assert!(Instant::now() < deadline, "{}", app.status);
            std::thread::yield_now();
        }
        assert_eq!(app.model.snapshot().nodes.len(), 2);
        assert_eq!(
            app.model
                .parameters
                .iter()
                .find(|p| p.info().id == format!("{id}.drive"))
                .unwrap()
                .get(),
            5.0
        );
        assert!(!app.device_running());
        drop(app);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn project_actions_roundtrip_without_opening_an_audio_device() {
        let temp = std::env::temp_dir().join(format!(
            "soniclaw-editor-workflow-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temp).unwrap();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut app = Workbench::new(None, 1, 0)?;
            let mut state = preset(1);
            state.assets.clear();
            state.name = "Loaded on worker".into();
            let json = temp.join("project.json");
            let original = state.to_json()?;
            std::fs::write(&json, &original)?;
            app.path = json.display().to_string();
            app.action(Action::Load);
            let poll = |app: &mut Workbench, condition: fn(&Workbench) -> bool| {
                let deadline = Instant::now() + Duration::from_secs(10);
                while condition(app) {
                    app.poll_project_work();
                    assert!(Instant::now() < deadline, "{}", app.status);
                    std::thread::yield_now();
                }
            };
            poll(&mut app, |a| a.loading);
            assert_eq!(app.model.snapshot().name, "Loaded on worker");
            let cached = app.model.build()?;
            assert_eq!(cached.project_state().unwrap().name, "Loaded on worker");
            app.recovery_directory = temp.join("recovery").display().to_string();
            app.action(Action::Checkpoint);
            let deadline = Instant::now() + Duration::from_secs(10);
            let store = soniclaw::project_storage::RecoveryStore::new(
                PathBuf::from(&app.recovery_directory),
                5,
            )?;
            while store.latest(soniclaw_workbench::APP_ID)?.is_none() {
                app.poll_project_work();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            app.single_bundle = temp.join("portable.soniclaw").display().to_string();
            app.action(Action::SingleBundle);
            poll(&mut app, |a| a.storage_worker.is_some());
            assert!(Path::new(&app.single_bundle).is_file(), "{}", app.status);
            app.import_directory = temp.join("import").display().to_string();
            app.action(Action::ImportBundle);
            poll(&mut app, |a| a.loading);
            assert_eq!(app.model.root, temp.join("import").canonicalize()?);
            app.action(Action::Recover);
            poll(&mut app, |a| a.loading);
            assert_eq!(app.model.root, temp.canonicalize()?);
            app.path = temp.join("missing.json").display().to_string();
            app.action(Action::Load);
            poll(&mut app, |a| a.loading);
            assert_eq!(app.model.snapshot().name, "Loaded on worker");
            assert!(app.status.starts_with("Load failed:"));
            assert!(!app.device_running());
            assert!(!app.autosave);
            assert_eq!(std::fs::read_to_string(json)?, original);
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(temp);
        result.unwrap();
    }
}
