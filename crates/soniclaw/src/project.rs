//! Control-thread project reconstruction with portable relative sample references.
use crate::{
    dsp::{AutomatedEffect, EffectKind},
    graph::Graph,
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
    sampler::{MultiSampler, SampleBank, Zone},
    state::*,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

fn build(
    state: &ProjectState,
    root: &Path,
    reuse: &[Arc<Parameter>],
    registry: &crate::registry::ProcessorRegistry,
) -> Result<(Engine, Vec<Arc<Parameter>>), StateError> {
    state.validate(&state.product_id)?;
    if state.nodes.is_empty() || state.outputs.is_empty() {
        return Err("a runnable project requires nodes and outputs".into());
    }
    let mut graph = Graph::default();
    let mut ids = BTreeMap::new();
    let mut parameters = vec![];
    let mut bank: Option<Arc<SampleBank>> = None;
    for node in &state.nodes {
        let processor: Box<dyn AudioProcessor> = if node.kind == ProcessorKind::Custom {
            let registered = registry.instantiate(node, state, root, reuse)?;
            parameters.extend(registered.parameters);
            registered.processor
        } else if node.kind == ProcessorKind::PolyModSynth {
            let mut synth = crate::dsp::voice_mod::PolyModSynth::<32>::new(
                &node.id,
                node.voice_modulation.clone().unwrap_or_default(),
            )?;
            synth.synth.set_tuning(state.tuning.clone().map(Arc::new))?;
            synth.reuse_parameters(reuse)?;
            validate_node_settings(
                node,
                synth.parameters(),
                &[
                    "mpe_master",
                    "mpe_first",
                    "mpe_last",
                    "mpe_member_bend",
                    "mpe_master_bend",
                ],
            )?;
            if node.settings.keys().any(|k| k.starts_with("mpe_")) {
                let get = |k, default| node.settings.get(k).copied().unwrap_or(default);
                let channel = |k, default| -> Result<u8, StateError> {
                    let v = get(k, default);
                    if !(0.0..=15.0).contains(&v) || v.fract() != 0.0 {
                        return Err("invalid MPE channel".into());
                    }
                    Ok(v as u8)
                };
                synth.synth.set_mpe_zone(Some(MpeZone {
                    master: channel("mpe_master", 0.0)?,
                    first_member: channel("mpe_first", 1.0)?,
                    last_member: channel("mpe_last", 15.0)?,
                    member_bend: get("mpe_member_bend", 48.0),
                    master_bend: get("mpe_master_bend", 2.0),
                }))?;
            }
            parameters.extend(synth.parameters().iter().cloned());
            Box::new(synth)
        } else if node.kind == ProcessorKind::Sampler {
            if node.settings.keys().any(|k| {
                ![
                    "attack_ms",
                    "release_ms",
                    "quality",
                    "mpe_master",
                    "mpe_first",
                    "mpe_last",
                    "mpe_member_bend",
                    "mpe_master_bend",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown sampler setting".into());
            }
            if bank.is_none() {
                let mut samples = vec![];
                let mut asset_ids = BTreeMap::new();
                let mut bytes = 0usize;
                for asset in &state.assets {
                    let file = crate::read_audio_with_limit(
                        state.resolve_asset(root, asset)?,
                        128 * 1024 * 1024 - bytes,
                    )?;
                    bytes = bytes
                        .checked_add(file.samples.len() * 4)
                        .ok_or("sample budget overflow")?;
                    if bytes > 128 * 1024 * 1024 {
                        return Err("project PCM exceeds 128 MiB".into());
                    }
                    asset_ids.insert(&asset.id, samples.len());
                    samples.push(Sample::new(
                        file.samples,
                        file.channels as usize,
                        file.sample_rate as f64,
                        asset.root_note,
                    )?);
                }
                let zones = state
                    .zones
                    .iter()
                    .map(|z| Zone {
                        release_trigger: z.release_trigger,
                        velocity_fade_in: z.velocity_fade_in,
                        velocity_fade_out: z.velocity_fade_out,
                        velocity_power: z.velocity_power,
                        sample: asset_ids[&z.asset],
                        low_key: z.keys[0],
                        high_key: z.keys[1],
                        low_velocity: z.velocities[0],
                        high_velocity: z.velocities[1],
                        round_robin_group: z.group,
                        gain: z.gain,
                        one_shot: z.one_shot,
                        loop_frames: z.loop_frames.map(|[a, b]| (a, b)),
                        start_frame: z.start_frame,
                        end_frame: z.end_frame,
                        loop_crossfade: z.loop_crossfade,
                    })
                    .collect();
                bank = Some(Arc::new(SampleBank::new(
                    samples,
                    zones,
                    128 * 1024 * 1024,
                )?));
            }
            let mut sampler = MultiSampler::<64>::new(bank.as_ref().expect("loaded bank").clone());
            sampler.set_tuning(state.tuning.clone().map(Arc::new))?;
            let get = |key: &str, default: f32| node.settings.get(key).copied().unwrap_or(default);
            sampler.set_envelope(get("attack_ms", 0.0), get("release_ms", 50.0))?;
            let quality = get("quality", 1.0);
            sampler.set_quality(match quality {
                0.0 => crate::dsp::resample::ResampleQuality::Linear,
                1.0 => crate::dsp::resample::ResampleQuality::SincFast,
                2.0 => crate::dsp::resample::ResampleQuality::SincBest,
                _ => return Err("invalid sampler quality".into()),
            });
            if node.settings.keys().any(|k| k.starts_with("mpe_")) {
                let channel = |key, default| -> Result<u8, StateError> {
                    let v = get(key, default);
                    if !(0.0..=15.0).contains(&v) || v.fract() != 0.0 {
                        return Err("invalid MPE channel".into());
                    }
                    Ok(v as u8)
                };
                sampler.set_mpe_zone(Some(MpeZone {
                    master: channel("mpe_master", 0.0)?,
                    first_member: channel("mpe_first", 1.0)?,
                    last_member: channel("mpe_last", 15.0)?,
                    member_bend: get("mpe_member_bend", 48.0),
                    master_bend: get("mpe_master_bend", 2.0),
                }))?;
            }
            Box::new(sampler)
        } else if node.kind == ProcessorKind::DiskSampler {
            if state.zones.iter().any(|z| {
                z.start_frame != 0
                    || z.end_frame.is_some()
                    || z.loop_crossfade != 0
                    || z.release_trigger
                    || z.velocity_fade_in.is_some()
                    || z.velocity_fade_out.is_some()
            }) {
                return Err("slice/crossfade zones require memory sampler".into());
            }
            if node.settings.keys().any(|k| {
                ![
                    "attack_ms",
                    "release_ms",
                    "quality",
                    "mpe_master",
                    "mpe_first",
                    "mpe_last",
                    "mpe_member_bend",
                    "mpe_master_bend",
                ]
                .contains(&k.as_str())
            }) {
                return Err("unknown disk sampler setting".into());
            }
            let assets = state
                .assets
                .iter()
                .map(|a| {
                    Ok(crate::stream::DiskAsset {
                        path: state.resolve_asset(root, a)?,
                        root_note: a.root_note,
                    })
                })
                .collect::<Result<Vec<_>, StateError>>()?;
            let asset_ids: BTreeMap<_, _> = state
                .assets
                .iter()
                .enumerate()
                .map(|(i, a)| (&a.id, i))
                .collect();
            let zones = state
                .zones
                .iter()
                .map(|z| crate::stream::DiskZone {
                    asset: asset_ids[&z.asset],
                    keys: z.keys,
                    velocities: z.velocities,
                    group: z.group as u8,
                    gain: z.gain,
                    one_shot: z.one_shot,
                    loop_frames: z.loop_frames.map(|[a, b]| (a as u64, b as u64)),
                })
                .collect();
            let (handle, mut sampler) =
                crate::stream::open_disk_sampler::<64>(assets, zones, 16 * 1024 * 1024)?;
            let get = |key: &str, default: f32| node.settings.get(key).copied().unwrap_or(default);
            sampler.set_envelope(get("attack_ms", 5.0), get("release_ms", 50.0))?;
            sampler.set_quality(match get("quality", 1.0) {
                0.0 => crate::dsp::resample::ResampleQuality::Linear,
                1.0 => crate::dsp::resample::ResampleQuality::SincFast,
                2.0 => crate::dsp::resample::ResampleQuality::SincBest,
                _ => return Err("invalid disk sampler quality".into()),
            });
            if node.settings.keys().any(|k| k.starts_with("mpe_")) {
                let channel = |key, default| -> Result<u8, StateError> {
                    let v = get(key, default);
                    if !(0.0..=15.0).contains(&v) || v.fract() != 0.0 {
                        return Err("invalid MPE channel".into());
                    }
                    Ok(v as u8)
                };
                sampler.set_mpe_zone(Some(MpeZone {
                    master: channel("mpe_master", 0.0)?,
                    first_member: channel("mpe_first", 1.0)?,
                    last_member: channel("mpe_last", 15.0)?,
                    member_bend: get("mpe_member_bend", 48.0),
                    master_bend: get("mpe_master_bend", 2.0),
                }))?;
            }
            Box::new(DiskProcessor {
                sampler,
                _handle: handle,
            })
        } else if matches!(
            node.kind,
            ProcessorKind::Granular | ProcessorKind::Convolution
        ) {
            let asset_id = node
                .asset
                .as_ref()
                .ok_or("resource node requires an asset ID")?;
            let asset = state
                .assets
                .iter()
                .find(|a| &a.id == asset_id)
                .ok_or("unknown asset")?;
            let file =
                crate::read_audio_with_limit(state.resolve_asset(root, asset)?, 128 * 1024 * 1024)?;
            if node.kind == ProcessorKind::Granular {
                let mut synth = crate::dsp::granular::GranularSynth::<128>::new(
                    Sample::new(
                        file.samples,
                        file.channels as usize,
                        file.sample_rate as f64,
                        asset.root_note,
                    )?,
                    &node.id,
                )?;
                synth.reuse_parameters(reuse)?;
                validate_node_settings(node, synth.parameters(), &[])?;
                parameters.extend(synth.parameters().iter().cloned());
                Box::new(synth)
            } else {
                let integer =
                    |key: &str, default: usize, max: usize| -> Result<usize, StateError> {
                        let value = node.settings.get(key).copied().unwrap_or(default as f32);
                        if value < 0.0 || value > max as f32 || value.fract() != 0.0 {
                            return Err("invalid convolution configuration".into());
                        }
                        Ok(value as usize)
                    };
                let partition = integer("partition", 0, 4096)?;
                let extra = integer("latency_frames", 0, 8192)?;
                let mode = if partition == 0 {
                    crate::dsp::stereo_convolution::ConvolutionMode::ZeroLatency
                } else {
                    crate::dsp::stereo_convolution::ConvolutionMode::Uniform { partition }
                };
                let impulse = crate::dsp::stereo_convolution::StereoImpulse::new(
                    file.samples,
                    file.channels as usize,
                    file.sample_rate as f64,
                )?;
                let mut effect = crate::dsp::stereo_convolution::ConvolutionEffect::new(
                    crate::dsp::stereo_convolution::StereoConvolution::new(impulse, mode, extra)?,
                    &node.id,
                )?;
                effect.reuse_parameters(reuse)?;
                validate_node_settings(
                    node,
                    effect.parameters(),
                    &["partition", "latency_frames"],
                )?;
                parameters.extend(effect.parameters().iter().cloned());
                Box::new(effect)
            }
        } else if node.kind == ProcessorKind::Wavetable {
            let bank = Arc::new(crate::dsp::advanced::wavetable::WavetableBank::basic(2048)?);
            let mut synth =
                crate::dsp::advanced::wavetable::WavetableSynth::<16>::new(bank, &node.id, 4)?;
            synth.reuse_parameters(reuse)?;
            for (key, value) in &node.settings {
                let id = format!("{}.{}", node.id, key);
                let p = synth
                    .parameters()
                    .iter()
                    .find(|p| p.info().id == id)
                    .ok_or("unknown wavetable control")?;
                let info = p.info();
                if !(info.min..=info.max).contains(value) {
                    return Err("wavetable setting out of range".into());
                }
            }
            parameters.extend(synth.parameters().iter().cloned());
            Box::new(synth)
        } else {
            let kind = match node.kind {
                ProcessorKind::Multiband => EffectKind::Multiband,
                ProcessorKind::DynamicEq => EffectKind::DynamicEq,
                ProcessorKind::SpectralFreeze => EffectKind::SpectralFreeze,
                ProcessorKind::Robotisation => EffectKind::Robotisation,
                ProcessorKind::PitchShift => EffectKind::PitchShift,
                ProcessorKind::FrequencyShift => EffectKind::FrequencyShift,
                ProcessorKind::StereoWidth => EffectKind::StereoWidth,
                ProcessorKind::StateVariable => EffectKind::StateVariable,
                ProcessorKind::Ladder => EffectKind::Ladder,
                ProcessorKind::CrossoverLow => EffectKind::CrossoverLow,
                ProcessorKind::CrossoverMid => EffectKind::CrossoverMid,
                ProcessorKind::CrossoverHigh => EffectKind::CrossoverHigh,
                ProcessorKind::RmsCompressor => EffectKind::RmsCompressor,
                ProcessorKind::LookaheadLimiter => EffectKind::LookaheadLimiter,
                ProcessorKind::TransientShaper => EffectKind::TransientShaper,
                ProcessorKind::SpectralGate => EffectKind::SpectralGate,
                ProcessorKind::FdnReverb => EffectKind::FdnReverb,
                ProcessorKind::Phaser => EffectKind::Phaser,
                ProcessorKind::Flanger => EffectKind::Flanger,
                ProcessorKind::Tremolo => EffectKind::Tremolo,
                ProcessorKind::AutoPan => EffectKind::AutoPan,
                ProcessorKind::MultiTapDelay => EffectKind::MultiTapDelay,
                ProcessorKind::Gain => EffectKind::Gain,
                ProcessorKind::LowPass => EffectKind::LowPass,
                ProcessorKind::Eq => EffectKind::Eq,
                ProcessorKind::Compressor => EffectKind::Compressor,
                ProcessorKind::Gate => EffectKind::Gate,
                ProcessorKind::Chorus => EffectKind::Chorus,
                ProcessorKind::Reverb => EffectKind::Reverb,
                ProcessorKind::Limiter => EffectKind::Limiter,
                ProcessorKind::Drive => EffectKind::Drive,
                ProcessorKind::Delay => EffectKind::Delay,
                ProcessorKind::Sampler
                | ProcessorKind::Custom
                | ProcessorKind::PolyModSynth
                | ProcessorKind::DiskSampler
                | ProcessorKind::Wavetable
                | ProcessorKind::Granular
                | ProcessorKind::Convolution => {
                    unreachable!()
                }
            };
            let mut effect = AutomatedEffect::new(kind, &node.id)?;
            effect.reuse_parameters(reuse)?;
            for (key, value) in &node.settings {
                let id = format!("{}.{}", node.id, key);
                let p = effect
                    .parameters()
                    .iter()
                    .find(|p| p.info().id == id)
                    .ok_or("unknown effect setting")?;
                let info = p.info();
                if !value.is_finite() || !(info.min..=info.max).contains(value) {
                    return Err("effect setting out of range".into());
                }
            }
            // Validation does not mutate reused controls until every node is constructed.
            parameters.extend(effect.parameters().iter().cloned());
            Box::new(effect)
        };
        let index = graph.add(Boxed(processor));
        graph.set_input(index, node.input_gain)?;
        ids.insert(&node.id, index);
    }
    for c in &state.connections {
        graph.connect(ids[&c.from], ids[&c.to], c.gain)?;
    }
    for (id, gain) in &state.outputs {
        graph.set_output(ids[id], *gain)?;
    }
    let mut processor: Box<dyn AudioProcessor> = Box::new(graph);
    if let Some(config) = &state.modulation {
        let matrix =
            crate::dsp::modulation::ModulationMatrix::new(config.clone(), &parameters, reuse)?;
        parameters.extend(matrix.parameters().iter().cloned());
        processor = Box::new(crate::dsp::modulation::ModulatedProcessor::new(
            processor, matrix,
        ));
    }
    if let Some(config) = &state.midi {
        processor = Box::new(crate::midi::performance::MidiProcessor::new(
            processor,
            config.clone(),
        ));
    }
    let mut values = BTreeMap::new();
    for p in &parameters {
        let info = p.info();
        let mut value = info.default;
        for node in &state.nodes {
            if let Some(key) = info.id.strip_prefix(&format!("{}.", node.id)) {
                if let Some(v) = node.settings.get(key) {
                    value = *v;
                }
            }
        }
        if let Some(v) = state.parameters.get(info.id) {
            value = *v;
        }
        if !value.is_finite() || !(info.min..=info.max).contains(&value) {
            return Err("project parameter out of range".into());
        }
        values.insert(info.id.to_owned(), value);
    }
    if state.parameters.keys().any(|id| !values.contains_key(id)) {
        return Err("unknown project parameter".into());
    }
    if parameters.len() > 4096 {
        return Err("project parameter count exceeds 4096".into());
    }
    let tempo_map = state
        .timeline
        .as_ref()
        .map(crate::transport::TempoMap::compile)
        .transpose()?
        .map(Arc::new);
    if let Some(document) = &state.automation {
        let mut automated =
            crate::automation::AutomatedProcessor::new(processor, document.clone(), &parameters)?.0;
        if let Some(map) = &tempo_map {
            automated = automated.with_tempo_map(map.clone());
        }
        processor = Box::new(automated);
    }
    if let (Some(timeline), Some(map)) = (&state.timeline, tempo_map) {
        processor = Box::new(crate::transport::TimelineProcessor::from_map(
            processor,
            timeline.clone(),
            map,
        )?);
    }
    // The caller publishes values only after successful reconstruction.
    let mut engine = Engine::default();
    engine.push(Boxed(processor));
    Ok((engine, parameters))
}
struct DiskProcessor {
    sampler: crate::stream::DiskSampler<64>,
    _handle: crate::stream::DiskHandle,
}
impl AudioProcessor for DiskProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.sampler.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        self.sampler.process(b, c)
    }
    fn reset(&mut self) {
        self.sampler.reset()
    }
}
struct Boxed(Box<dyn AudioProcessor>);
impl AudioProcessor for Boxed {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.0.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        self.0.process(b, c)
    }
    fn reset(&mut self) {
        self.0.reset()
    }
    fn latency_frames(&self) -> usize {
        self.0.latency_frames()
    }
}
fn apply_values(state: &ProjectState, params: &[Arc<Parameter>]) -> Result<(), StateError> {
    for p in params {
        let info = p.info();
        let value = state
            .parameters
            .get(info.id)
            .copied()
            .or_else(|| {
                state.nodes.iter().find_map(|n| {
                    info.id
                        .strip_prefix(&format!("{}.", n.id))
                        .and_then(|k| n.settings.get(k))
                        .copied()
                })
            })
            .unwrap_or(info.default);
        p.set(value)?;
    }
    Ok(())
}
/// Build a plug-in/rack off-thread. Structural host-state restore is supported
/// while deactivated with the same parameter identities; scalar state also loads live.
pub fn create_plugin(
    descriptor: PluginDescriptor,
    state: ProjectState,
    asset_root: impl Into<PathBuf>,
) -> Result<PluginInstance, StateError> {
    create_plugin_with_registry(
        descriptor,
        state,
        asset_root,
        Arc::new(crate::registry::ProcessorRegistry::default()),
    )
}
/// Construct custom nodes using a frozen, application-owned compiled DSP registry.
pub fn create_plugin_with_registry(
    descriptor: PluginDescriptor,
    state: ProjectState,
    asset_root: impl Into<PathBuf>,
    registry: Arc<crate::registry::ProcessorRegistry>,
) -> Result<PluginInstance, StateError> {
    state.validate(descriptor.id)?;
    let state = registry.normalize_project(&state)?;
    let root = asset_root.into();
    let (engine, params) = build(&state, &root, &[], &registry)?;
    apply_values(&state, &params)?;
    let plugin = PluginInstance::new(descriptor, engine, params)?;
    let normalizer = registry.clone();
    plugin
        .with_project(state, move |state, reuse| {
            let (engine, params) = build(state, &root, reuse, &registry)?;
            if params.len() != reuse.len() {
                return Err("host parameter set must remain stable".into());
            }
            Ok(engine)
        })?
        .with_project_normalizer(move |s| normalizer.normalize_project(s))
}

/// Standalone document editing is transactional: build all resources before
/// swapping the document and engine. Stop the device before calling edit/undo/redo.
pub struct ProjectSession {
    history: ProjectHistory,
    asset_root: PathBuf,
    registry: Arc<crate::registry::ProcessorRegistry>,
}
impl ProjectSession {
    pub fn new(
        state: ProjectState,
        asset_root: impl Into<PathBuf>,
        undo_capacity: usize,
    ) -> Result<Self, StateError> {
        Self::with_registry(
            state,
            asset_root,
            undo_capacity,
            Arc::new(crate::registry::ProcessorRegistry::default()),
        )
    }
    pub fn with_registry(
        state: ProjectState,
        asset_root: impl Into<PathBuf>,
        undo_capacity: usize,
        registry: Arc<crate::registry::ProcessorRegistry>,
    ) -> Result<Self, StateError> {
        let state = registry.normalize_project(&state)?;
        Ok(Self {
            registry,
            history: ProjectHistory::new(state, undo_capacity)?,
            asset_root: asset_root.into(),
        })
    }
    pub fn history(&self) -> &ProjectHistory {
        &self.history
    }
    pub fn edit(
        &mut self,
        descriptor: PluginDescriptor,
        label: &str,
        edit: impl FnOnce(&mut ProjectState),
    ) -> Result<PluginInstance, StateError> {
        let mut next = self.history.clone();
        let mut candidate = next.current().clone();
        edit(&mut candidate);
        let candidate = self.registry.normalize_project(&candidate)?;
        next.edit(label, |s| *s = candidate)?;
        let plugin = create_plugin_with_registry(
            descriptor,
            next.current().clone(),
            self.asset_root.clone(),
            self.registry.clone(),
        )?;
        self.history = next;
        Ok(plugin)
    }
    pub fn undo(
        &mut self,
        descriptor: PluginDescriptor,
    ) -> Result<Option<PluginInstance>, StateError> {
        let mut next = self.history.clone();
        if !next.undo() {
            return Ok(None);
        }
        let plugin = create_plugin_with_registry(
            descriptor,
            next.current().clone(),
            self.asset_root.clone(),
            self.registry.clone(),
        )?;
        self.history = next;
        Ok(Some(plugin))
    }
    pub fn redo(
        &mut self,
        descriptor: PluginDescriptor,
    ) -> Result<Option<PluginInstance>, StateError> {
        let mut next = self.history.clone();
        if !next.redo() {
            return Ok(None);
        }
        let plugin = create_plugin_with_registry(
            descriptor,
            next.current().clone(),
            self.asset_root.clone(),
            self.registry.clone(),
        )?;
        self.history = next;
        Ok(Some(plugin))
    }
    pub fn build(&self, descriptor: PluginDescriptor) -> Result<PluginInstance, StateError> {
        create_plugin_with_registry(
            descriptor,
            self.history.current().clone(),
            self.asset_root.clone(),
            self.registry.clone(),
        )
    }
}

fn validate_node_settings(
    node: &RackNode,
    params: &[Arc<Parameter>],
    structural: &[&str],
) -> Result<(), StateError> {
    for (key, value) in &node.settings {
        if structural.contains(&key.as_str()) {
            continue;
        }
        let id = format!("{}.{}", node.id, key);
        let p = params
            .iter()
            .find(|p| p.info().id == id)
            .ok_or("unknown node setting")?;
        if !value.is_finite() || !(p.info().min..=p.info().max).contains(value) {
            return Err("node setting out of range".into());
        }
    }
    Ok(())
}

/// Control-built project for prepared graphs. Stop before final resource destruction.
pub struct ProjectProcessor {
    plugin: PluginInstance,
    prepared: Option<ProcessSpec>,
}
impl ProjectProcessor {
    pub fn new(plugin: PluginInstance) -> Self {
        Self {
            plugin,
            prepared: None,
        }
    }
    pub(crate) fn from_prepared(plugin: PluginInstance, prepared: Option<ProcessSpec>) -> Self {
        Self { plugin, prepared }
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        self.plugin.parameters()
    }
}
impl AudioProcessor for ProjectProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if self.prepared.is_some_and(|p| {
            p.sample_rate == s.sample_rate
                && p.max_block_frames == s.max_block_frames
                && p.channels == s.channels
        }) {
            return Ok(());
        }
        self.prepared = None;
        self.plugin.activate(s)?;
        self.prepared = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if self.plugin.set_transport(c.transport).is_err()
            || self.plugin.process(b.samples_mut(), c.events).is_err()
        {
            b.clear();
        }
    }
    fn reset(&mut self) {
        self.plugin.reset();
    }
    fn latency_frames(&self) -> usize {
        self.plugin.latency_frames()
    }
}
