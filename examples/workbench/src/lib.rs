//! Native workbench model, offline renderer and CLAP reference factories.
use soniclaw::{
    dsp::{
        advanced::spectral::{SpectrumAnalyzer, WindowKind},
        modulation::*,
    },
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
    state::*,
};
use soniclaw_clap::{descriptor, Definition};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
pub const APP_ID: &str = "org.soniclaw.workbench";
/// Freeze registrations once; every editor, loader, renderer and restore uses this registry.
pub fn registry() -> Arc<soniclaw::registry::ProcessorRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<soniclaw::registry::ProcessorRegistry>> =
        std::sync::OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let mut r = soniclaw::registry::ProcessorRegistry::default();
            soniclaw_example_processors::register(&mut r).expect("valid example registrations");
            Arc::new(r)
        })
        .clone()
}
pub fn create_plugin(
    descriptor: PluginDescriptor,
    state: ProjectState,
    root: impl Into<PathBuf>,
) -> Result<PluginInstance, StateError> {
    soniclaw::project::create_plugin_with_registry(descriptor, state, root, registry())
}
pub fn default_root() -> PathBuf {
    std::env::var_os("SONICLAW_WORKBENCH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}
fn source(kind: ProcessorKind) -> bool {
    matches!(
        kind,
        ProcessorKind::PolyModSynth
            | ProcessorKind::Granular
            | ProcessorKind::Wavetable
            | ProcessorKind::Sampler
            | ProcessorKind::DiskSampler
    )
}
fn node_source(n: &RackNode) -> bool {
    source(n.kind)
        || n.custom
            .as_ref()
            .and_then(|c| registry().metadata(&c.processor_id))
            .is_some_and(|m| m.instrument)
}
pub fn is_instrument(state: &ProjectState) -> bool {
    state.nodes.iter().any(node_source)
}
pub fn app_descriptor(state: &ProjectState) -> PluginDescriptor {
    PluginDescriptor {
        id: APP_ID,
        name: "Soniclaw Workbench",
        vendor: "Soniclaw",
        version: env!("CARGO_PKG_VERSION"),
        is_instrument: is_instrument(state),
        accepts_midi: is_instrument(state),
    }
}
pub fn preset(index: usize) -> ProjectState {
    if index == 6 {
        let mut p = ProjectState::empty(APP_ID, "Tape Colour Rack");
        let mut node = registry()
            .node("tape", soniclaw_example_processors::TAPE_ID)
            .expect("registered tape");
        node.input_gain = 1.0;
        p.nodes.push(node);
        p.outputs.insert("tape".into(), 1.0);
        return p;
    }
    let mut p = ProjectState::empty(
        APP_ID,
        [
            "Motion Rack",
            "Multiband Glue",
            "Granular Cloud",
            "Spectral Space",
            "Poly Motion",
            "Sampler Studio",
        ][index.min(5)],
    );
    let nodes = match index {
        4 => vec![
            ("voice", ProcessorKind::PolyModSynth),
            ("space", ProcessorKind::FdnReverb),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
        5 => vec![
            ("sampler", ProcessorKind::Sampler),
            ("tone", ProcessorKind::StateVariable),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
        1 => vec![
            ("band", ProcessorKind::Multiband),
            ("dynamic", ProcessorKind::DynamicEq),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
        2 => vec![
            ("grain", ProcessorKind::Granular),
            ("tone", ProcessorKind::StateVariable),
            ("space", ProcessorKind::FdnReverb),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
        3 => vec![
            ("osc", ProcessorKind::Wavetable),
            ("freeze", ProcessorKind::SpectralFreeze),
            ("ir", ProcessorKind::Convolution),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
        _ => vec![
            ("osc", ProcessorKind::Wavetable),
            ("tone", ProcessorKind::StateVariable),
            ("band", ProcessorKind::Multiband),
            ("space", ProcessorKind::FdnReverb),
            ("limit", ProcessorKind::LookaheadLimiter),
        ],
    };
    for (i, (id, kind)) in nodes.iter().enumerate() {
        let mut n = RackNode::new(*id, *kind);
        n.input_gain = if i == 0 && !source(*kind) { 1.0 } else { 0.0 };
        if *kind == ProcessorKind::Granular {
            n.asset = Some("grain-source".into());
        }
        if *kind == ProcessorKind::Convolution {
            n.asset = Some("stereo-ir".into());
        }
        p.nodes.push(n);
    }
    if index == 4 {
        use soniclaw::dsp::voice_mod::*;
        p.nodes[0].voice_modulation = Some(VoiceModulationConfig {
            routes: vec![
                VoiceRoute {
                    source: VoiceSource::Envelope2,
                    destination: VoiceDestination::Cutoff,
                    amount: 0.45,
                },
                VoiceRoute {
                    source: VoiceSource::Lfo1,
                    destination: VoiceDestination::Pitch,
                    amount: 0.006,
                },
                VoiceRoute {
                    source: VoiceSource::Velocity,
                    destination: VoiceDestination::Level,
                    amount: 0.25,
                },
                VoiceRoute {
                    source: VoiceSource::Pressure,
                    destination: VoiceDestination::Cutoff,
                    amount: 0.3,
                },
            ],
            ..Default::default()
        });
    }
    if index == 5 {
        let mut z = SampleZone::new("grain-source");
        z.keys = [36, 84];
        z.one_shot = false;
        z.loop_frames = Some([4800, 48000]);
        z.loop_crossfade = 512;
        p.zones.push(z);
    }
    if index == 1 {
        for band in ["low", "mid", "high"] {
            p.nodes[0].settings.insert(format!("{band}_ratio"), 2.0);
        }
    }
    if index == 0 || index == 2 {
        p.modulation = Some(ModulationConfig {
            sources: vec![
                SourceConfig::Lfo {
                    shape: LfoShape::Sine,
                    rate_hz: 0.5,
                    beats: Some(4.0),
                },
                SourceConfig::Macro { index: 0 },
            ],
            routes: vec![
                ModulationRoute {
                    source: 0,
                    destination: "tone.frequency".into(),
                    amount: 0.08,
                },
                ModulationRoute {
                    source: 1,
                    destination: "space.mix".into(),
                    amount: 0.2,
                },
            ],
            macros: vec![0.5],
        });
    }
    p.assets = vec![
        Asset {
            id: "grain-source".into(),
            path: "assets/grain-source.wav".into(),
            root_note: 57,
        },
        Asset {
            id: "stereo-ir".into(),
            path: "assets/stereo-ir.wav".into(),
            root_note: 60,
        },
    ];
    serialise_chain(&mut p);
    p
}
pub fn serialise_chain(p: &mut ProjectState) {
    p.connections.clear();
    p.outputs.clear();
    for i in 0..p.nodes.len() {
        p.nodes[i].input_gain = if i == 0 && !node_source(&p.nodes[i]) {
            1.0
        } else {
            0.0
        };
        if i > 0 {
            p.connections.push(Connection {
                from: p.nodes[i - 1].id.clone(),
                to: p.nodes[i].id.clone(),
                gain: 1.0,
            });
        }
    }
    if let Some(last) = p.nodes.last() {
        p.outputs.insert(last.id.clone(), 1.0);
    }
}
pub struct Model {
    pub history: ProjectHistory,
    pub root: PathBuf,
    pub parameters: Vec<Arc<Parameter>>,
    loaded_cache: Option<(ProjectState, PluginInstance)>,
}
impl Model {
    pub fn new(index: usize, root: PathBuf) -> Result<Self, StateError> {
        let state = preset(index);
        let plugin = create_plugin(app_descriptor(&state), state.clone(), &root)?;
        Ok(Self {
            loaded_cache: None,
            history: ProjectHistory::new(state, 64)?,
            root,
            parameters: plugin.parameters().to_vec(),
        })
    }
    pub fn snapshot(&self) -> ProjectState {
        let mut p = self.history.current().clone();
        p.parameters = self
            .parameters
            .iter()
            .map(|p| (p.info().id.to_owned(), p.get()))
            .collect();
        p
    }
    pub fn record_controls(&mut self, label: &str) -> Result<(), StateError> {
        let values = self
            .parameters
            .iter()
            .map(|p| (p.info().id.to_owned(), p.get()))
            .collect();
        self.history.edit(label, |s| s.parameters = values)
    }
    pub fn apply(&mut self, mut candidate: ProjectState, label: &str) -> Result<(), StateError> {
        candidate.validate(APP_ID)?;
        let plugin = create_plugin(app_descriptor(&candidate), candidate.clone(), &self.root)?;
        candidate = plugin.project_state().cloned().unwrap_or(candidate);
        candidate.parameters = plugin
            .parameters()
            .iter()
            .map(|p| (p.info().id.to_owned(), p.get()))
            .collect();
        self.history.edit(label, |p| *p = candidate)?;
        self.loaded_cache = None;
        self.parameters = plugin.parameters().to_vec();
        Ok(())
    }
    pub fn edit_chain(&mut self, nodes: Vec<RackNode>, label: &str) -> Result<(), StateError> {
        let mut p = self.snapshot();
        p.nodes = nodes;
        p.layout.retain(|id, _| p.nodes.iter().any(|n| &n.id == id));
        serialise_chain(&mut p);
        let ids: Vec<_> = p.nodes.iter().map(|n| n.id.clone()).collect();
        p.parameters.retain(|key, _| {
            key.starts_with("mod.") || ids.iter().any(|id| key.starts_with(&format!("{id}.")))
        });
        if let Some(m) = &mut p.modulation {
            m.routes.retain(|r| {
                ids.iter()
                    .any(|id| r.destination.starts_with(&format!("{id}.")))
            });
        }
        self.apply(p, label)
    }
    pub fn apply_routing(&mut self, d: &ProjectState) -> Result<(), StateError> {
        let mut s = self.snapshot();
        s.connections = d.connections.clone();
        s.outputs = d.outputs.clone();
        s.layout = d.layout.clone();
        for n in &mut s.nodes {
            if let Some(a) = d.nodes.iter().find(|a| a.id == n.id) {
                n.input_gain = a.input_gain;
            }
        }
        self.apply(s, "Edit routing")
    }
    pub fn apply_samples(&mut self, d: &ProjectState) -> Result<(), StateError> {
        let mut s = self.snapshot();
        s.assets = d.assets.clone();
        s.zones = d.zones.clone();
        self.apply(s, "Edit sample map")
    }
    pub fn undo(&mut self) -> Result<bool, StateError> {
        let mut next = self.history.clone();
        if !next.undo() {
            return Ok(false);
        }
        let plugin = create_plugin(
            app_descriptor(next.current()),
            next.current().clone(),
            &self.root,
        )?;
        self.history = next;
        self.loaded_cache = None;
        self.parameters = plugin.parameters().to_vec();
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool, StateError> {
        let mut next = self.history.clone();
        if !next.redo() {
            return Ok(false);
        }
        let plugin = create_plugin(
            app_descriptor(next.current()),
            next.current().clone(),
            &self.root,
        )?;
        self.history = next;
        self.loaded_cache = None;
        self.parameters = plugin.parameters().to_vec();
        Ok(true)
    }
    pub fn build(&mut self) -> Result<PluginInstance, StateError> {
        if let Some((state, plugin)) = self.loaded_cache.take() {
            if &state == self.history.current() {
                return Ok(plugin);
            }
        }
        let state = self.snapshot();
        let p = create_plugin(app_descriptor(&state), state, &self.root)?;
        self.parameters = p.parameters().to_vec();
        Ok(p)
    }
    /// Commit a successfully constructed worker result without reopening assets.
    pub fn install_loaded(
        &mut self,
        loaded: soniclaw::project_loading::LoadedProject,
    ) -> Result<(), StateError> {
        loaded.state().validate(APP_ID)?;
        let (mut state, root, plugin) = loaded.into_parts();
        if plugin.descriptor.id != app_descriptor(&state).id {
            return Err("loaded product mismatch".into());
        }
        state.parameters = plugin
            .parameters()
            .iter()
            .map(|p| (p.info().id.to_owned(), p.get()))
            .collect();
        let history = ProjectHistory::new(state.clone(), 64)?;
        self.parameters = plugin.parameters().to_vec();
        self.history = history;
        self.root = root;
        self.loaded_cache = Some((state, plugin));
        Ok(())
    }
    pub fn save(&mut self, path: &Path) -> Result<(), StateError> {
        self.record_controls("Save controls")?;
        let p = self.snapshot();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if parent.canonicalize()? != self.root.canonicalize()? {
            return Err("save the JSON in the project root, or export a portable bundle".into());
        }
        std::fs::write(path, p.to_json()?)?;
        Ok(())
    }
    pub fn load(&mut self, path: &Path) -> Result<(), StateError> {
        let state = ProjectState::from_json(&std::fs::read_to_string(path)?)?;
        state.validate(APP_ID)?;
        let root = path.parent().unwrap_or(Path::new(".")).canonicalize()?;
        let plugin = create_plugin(app_descriptor(&state), state.clone(), &root)?;
        self.loaded_cache = None;
        self.history = ProjectHistory::new(state, 64)?;
        self.root = root;
        self.parameters = plugin.parameters().to_vec();
        Ok(())
    }
    pub fn export_bundle(&self, directory: &Path) -> Result<(), StateError> {
        let mut p = self.snapshot();
        let mut sources = vec![];
        for a in &p.assets {
            sources.push(p.resolve_asset(&self.root, a)?);
        }
        std::fs::create_dir(directory)?;
        std::fs::create_dir(directory.join("assets"))?;
        for (i, (asset, file)) in p.assets.iter_mut().zip(sources).enumerate() {
            let name = format!("assets/sample-{i}.wav");
            std::fs::copy(file, directory.join(&name))?;
            asset.path = name;
        }
        std::fs::write(directory.join("project.json"), p.to_json()?)?;
        Ok(())
    }
}
pub struct Preview {
    pub audio: Vec<f32>,
    pub waveform: Vec<f32>,
    pub spectrum: Vec<f32>,
    pub cpu_percent: f32,
    pub latency: usize,
    pub wall_ms: f64,
    pub frames: usize,
}
pub fn render(state: ProjectState, root: PathBuf, seconds: f64) -> Result<Preview, String> {
    render_inner(state, root, seconds).map_err(|e| e.to_string())
}
fn render_inner(state: ProjectState, root: PathBuf, seconds: f64) -> Result<Preview, StateError> {
    if !seconds.is_finite() || !(0.1..=10.0).contains(&seconds) {
        return Err("preview duration must be 0.1–10 seconds".into());
    }
    let instrument = is_instrument(&state);
    let mut plugin = create_plugin(app_descriptor(&state), state, root)?;
    plugin.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    })?;
    let latency = plugin.latency_frames();
    let frames = (48000.0 * seconds).round() as usize;
    let mut audio = vec![0.0; frames * 2];
    let mut cpu = Duration::ZERO;
    let release_at = frames.saturating_sub(9600);
    for (start, block) in audio.chunks_mut(512).enumerate() {
        let absolute = start * 256;
        if !instrument {
            for (i, f) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let t = (absolute + i) as f64 / 48000.0;
                let x = (std::f64::consts::TAU * 220.0 * t).sin() as f32 * 0.12;
                f.fill(x);
            }
        }
        let mut events = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; 3];
        let count = if instrument && start == 0 {
            for (i, note) in [60, 64, 67].into_iter().enumerate() {
                events[i] = Event {
                    frame: 0,
                    message: MidiMessage::NoteOnId {
                        channel: 1,
                        note,
                        velocity: 0.65,
                        note_id: i as i32 + 1,
                    },
                };
            }
            3
        } else if instrument && absolute <= release_at && release_at < absolute + block.len() / 2 {
            events[0] = Event {
                frame: release_at - absolute,
                message: MidiMessage::ControlChange {
                    channel: 1,
                    controller: 123,
                    value: 0,
                },
            };
            1
        } else {
            0
        };
        let now = Instant::now();
        plugin.process(block, &events[..count])?;
        cpu += now.elapsed();
    }
    let waveform = audio
        .as_chunks::<2>()
        .0
        .iter()
        .step_by((frames / 600).max(1))
        .map(|f| (f[0] + f[1]) * 0.5)
        .collect();
    let mut mono = [0.0; 1024];
    let start = (latency + 4096).min(frames.saturating_sub(1024));
    for (i, x) in mono.iter_mut().enumerate() {
        let index = (start + i).min(frames - 1);
        *x = (audio[index * 2] + audio[index * 2 + 1]) * 0.5;
    }
    let spectrum = SpectrumAnalyzer::new(1024, WindowKind::Hann)?
        .analyze(&mono)?
        .to_vec();
    Ok(Preview {
        audio,
        waveform,
        spectrum,
        cpu_percent: (cpu.as_secs_f64() / seconds * 100.0) as f32,
        latency,
        wall_ms: cpu.as_secs_f64() * 1000.0,
        frames,
    })
}
pub fn demonstration(directory: &Path) -> Result<(), StateError> {
    std::fs::create_dir(directory)?;
    let root = default_root();
    std::fs::create_dir(directory.join("assets"))?;
    for name in ["grain-source.wav", "stereo-ir.wav"] {
        std::fs::copy(
            root.join("assets").join(name),
            directory.join("assets").join(name),
        )?;
    }
    let mut results = BTreeMap::new();
    for index in 0..6 {
        let state = preset(index);
        let preview = render(state.clone(), directory.to_path_buf(), 2.0)
            .map_err(|e| -> StateError { e.into() })?;
        soniclaw::io::write_wav(
            directory.join(format!("preset-{index}.wav")),
            48000,
            2,
            &preview.audio,
        )?;
        std::fs::write(
            directory.join(format!("preset-{index}.json")),
            state.to_json()?,
        )?;
        results.insert(state.name,serde_json::json!({"cpu_percent_of_realtime":preview.cpu_percent,"latency_frames":preview.latency,"frames":preview.frames}));
    }
    let sample = soniclaw::io::read_wav(root.join("assets/grain-source.wav"))?;
    let stretched =
        soniclaw::dsp::spectral_fx::time_stretch_interleaved(&sample.samples, 1, 1.5, 1024)?;
    soniclaw::io::write_wav(directory.join("time-stretch-150.wav"), 48000, 1, &stretched)?;
    let shifted =
        soniclaw::dsp::spectral_fx::pitch_shift_interleaved(&sample.samples, 1, 48000.0, 7.0)?;
    soniclaw::io::write_wav(directory.join("pitch-shift-7.wav"), 48000, 1, &shifted)?;
    // Prepare replacement kernels on the control side; process only swaps pointers.
    use soniclaw::dsp::stereo_convolution::*;
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    };
    let (mut control, mut convolution) = convolution_pair(
        StereoImpulse::new(vec![1.0], 1, 48000.0)?,
        ConvolutionMode::ZeroLatency,
        0,
        2048,
        spec,
    )?;
    let wav = soniclaw::io::read_wav(root.join("assets/stereo-ir.wav"))?;
    let replacement =
        StereoImpulse::new(wav.samples, wav.channels as usize, wav.sample_rate as f64)?;
    let mut swapped = vec![0.0; 96000 * 2];
    for (block, data) in swapped.chunks_mut(512).enumerate() {
        if block == 187 {
            control.replace(replacement.clone())?;
        }
        for (i, frame) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            frame.fill(
                0.1 * (std::f32::consts::TAU * 220.0 * (block * 256 + i) as f32 / 48000.0).sin(),
            );
        }
        convolution.process(
            &mut AudioBuffer::new(data, 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    let retired = control.collect_retired();
    soniclaw::io::write_wav(directory.join("ir-crossfade.wav"), 48000, 2, &swapped)?;
    results.insert("IR crossfade".into(), serde_json::json!({"latency_frames":convolution.latency_frames(),"fade_frames":2048,"retired_on_control":retired}));
    std::fs::write(
        directory.join("render-report.json"),
        serde_json::to_string_pretty(&results)?,
    )?;
    println!("Rendered six workbench presets, granular/modulated/multiband/spectral/convolution projects and offline pitch/time examples into {}",directory.display());
    Ok(())
}
fn clap_effect() -> PluginInstance {
    let mut p = preset(1);
    p.product_id = "org.soniclaw.workbench.fx".into();
    create_plugin(
        PluginDescriptor {
            id: "org.soniclaw.workbench.fx",
            name: "Soniclaw Multiband Workbench",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: false,
            accepts_midi: false,
        },
        p,
        default_root(),
    )
    .expect("valid factory")
}
fn clap_granular() -> PluginInstance {
    let mut p = preset(2);
    p.product_id = "org.soniclaw.workbench.granular".into();
    create_plugin(
        PluginDescriptor {
            id: "org.soniclaw.workbench.granular",
            name: "Soniclaw Granular Workbench",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: true,
            accepts_midi: true,
        },
        p,
        default_root(),
    )
    .expect("valid factory and bundled assets")
}
static FX: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.workbench.fx",
    c"Soniclaw Multiband Workbench",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static GRAIN: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.workbench.granular",
    c"Soniclaw Granular Workbench",
    c"Soniclaw",
    c"0.146.0",
    true,
);
soniclaw_clap::export_clap!([
    Definition {
        descriptor: &FX,
        create: clap_effect,
        instrument: false,
        tail_seconds: 1.0
    },
    Definition {
        descriptor: &GRAIN,
        create: clap_granular,
        instrument: true,
        tail_seconds: 4.0
    },
    Definition {
        descriptor: &POLY,
        create: clap_poly,
        instrument: true,
        tail_seconds: 4.0
    },
    Definition {
        descriptor: &SAMPLER,
        create: clap_sampler,
        instrument: true,
        tail_seconds: 4.0
    },
]);

fn clap_poly() -> PluginInstance {
    product(4, "org.soniclaw.workbench.polymod", "Soniclaw Poly Motion")
}
fn clap_sampler() -> PluginInstance {
    product(
        5,
        "org.soniclaw.workbench.sampler",
        "Soniclaw Sampler Studio",
    )
}
fn product(index: usize, id: &'static str, name: &'static str) -> PluginInstance {
    let mut s = preset(index);
    s.product_id = id.into();
    create_plugin(
        PluginDescriptor {
            id,
            name,
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: true,
            accepts_midi: true,
        },
        s,
        default_root(),
    )
    .expect("valid factory")
}
static POLY: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.workbench.polymod",
    c"Soniclaw Poly Motion",
    c"Soniclaw",
    c"0.146.0",
    true,
);
static SAMPLER: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.workbench.sampler",
    c"Soniclaw Sampler Studio",
    c"Soniclaw",
    c"0.146.0",
    true,
);

pub struct HostedPreview {
    pub audio: Vec<f32>,
    pub latency: usize,
    pub state: Vec<u8>,
    pub parameters: Vec<soniclaw::host::ParameterInfo>,
    pub output_events: usize,
    pub processing_ms: f64,
}
/// # Safety
/// Path must name explicitly trusted native CLAP code obeying the ABI/threading contract.
pub unsafe fn render_hosted(
    path: &Path,
    id: &str,
    values: &[soniclaw::host::ParameterInfo],
    restore: Option<&[u8]>,
) -> Result<HostedPreview, soniclaw::host::HostError> {
    // SAFETY: caller explicitly selects trusted code. Owners remain on this main thread.
    let library = unsafe { soniclaw::host::ClapLibrary::load(path) }?;
    let mut p = library.instantiate(id)?;
    if let Some(s) = restore {
        p.load_state(s)?;
    }
    let (input, channels) = p.main_channels();
    let mut audio = vec![0.0; 96000 * channels];
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels,
    })?;
    let latency = p.latency_frames();
    let automation: Vec<_> = values
        .iter()
        .filter(|v| {
            v.flags & 8 == 0
                && p.parameters()
                    .iter()
                    .find(|q| q.id == v.id)
                    .is_some_and(|q| q.value != v.value)
        })
        .map(|v| soniclaw::host::ParameterEvent {
            frame: 0,
            id: v.id,
            value: v.value,
        })
        .collect();
    let (emitted, ms) = p.run_audio(|a| -> Result<(usize, f64), Error> {
        let mut emitted = 0;
        let mut cpu = Duration::ZERO;
        for (i, b) in audio.chunks_mut(channels * 256).enumerate() {
            let at = i * 256;
            let mut e = [Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            }];
            let mut n = 0;
            if input == 0 && at == 0 {
                e[0].message = MidiMessage::NoteOnId {
                    channel: 1,
                    note: 60,
                    velocity: 0.7,
                    note_id: 41,
                };
                n = 1;
            }
            if input == 0 && at == 76800 {
                e[0].message = MidiMessage::NoteOffId {
                    target: NoteTarget {
                        note_id: 41,
                        channel: 1,
                        key: 60,
                    },
                };
                n = 1;
            }
            if input > 0 {
                for (j, f) in b.chunks_exact_mut(channels).enumerate() {
                    f.fill(
                        0.12 * (std::f32::consts::TAU * 220.0 * (at + j) as f32 / 48000.0).sin(),
                    );
                }
            }
            let start = Instant::now();
            a.process_block(
                b,
                &e[..n],
                if at == 0 { &automation } else { &[] },
                Transport {
                    sample_position: at as u64,
                    tempo: 120.0,
                    playing: true,
                },
            )?;
            cpu += start.elapsed();
            emitted += a.output_events().len();
        }
        Ok((emitted, cpu.as_secs_f64() * 1000.0))
    })??;
    p.service_main_thread();
    p.deactivate();
    let state = p.save_state()?;
    p.load_state(&state)?;
    let parameters = p.parameters().to_vec();
    Ok(HostedPreview {
        audio,
        latency,
        state,
        parameters,
        output_events: emitted,
        processing_ms: ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_edits_undo_bundle_and_reload() {
        let root = default_root();
        let mut m = Model::new(0, root).unwrap();
        let mut nodes = m.snapshot().nodes;
        nodes.swap(2, 3);
        m.edit_chain(nodes, "Move").unwrap();
        assert_eq!(m.history.current().nodes[2].id, "space");
        m.undo().unwrap();
        assert_eq!(m.history.current().nodes[2].id, "band");
        m.redo().unwrap();
        let path = std::env::temp_dir().join(format!("soniclaw-bundle-{}", std::process::id()));
        m.export_bundle(&path).unwrap();
        let state = m.snapshot();
        m.load(&path.join("project.json")).unwrap();
        assert_eq!(m.snapshot().nodes, state.nodes);
        drop(m);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn all_presets_render_finite_audio() {
        for i in 0..6 {
            let p = render(preset(i), default_root(), 0.25).unwrap();
            assert!(p.audio.iter().all(|x| x.is_finite()));
            assert!(p.audio.iter().any(|x| x.abs() > 0.00001));
        }
    }
}
