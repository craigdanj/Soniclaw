//! Format-independent plug-in lifecycle and stable parameter identity. The
//! soniclaw-clap crate wraps this ownership boundary with an exported CLAP ABI.
use soniclaw_bus::{BusEngine, BusLayout, BusProcessor, BusSpec};
use soniclaw_core::*;
use soniclaw_engine::{Engine, Engine64};
use soniclaw_state::{Preset, ProjectState, StateError};
type ProjectFactory =
    dyn Fn(&ProjectState, &[Arc<Parameter>]) -> Result<Engine, StateError> + Send + Sync;
type ProjectNormalizer = dyn Fn(&ProjectState) -> Result<ProjectState, StateError> + Send + Sync;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct PluginDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub version: &'static str,
    pub is_instrument: bool,
    pub accepts_midi: bool,
}
pub struct PluginInstance {
    pub descriptor: PluginDescriptor,
    engine: Engine,
    engine64: Option<Engine64>,
    precision_scratch: Vec<f64>,
    parameters: Vec<Arc<Parameter>>,
    active: bool,
    project: Option<ProjectState>,
    project_factory: Option<Arc<ProjectFactory>>,
    project_normalizer: Option<Arc<ProjectNormalizer>>,
    sidechain: Option<BusEngine>,
    sidechain_input: Vec<f32>,
}
impl PluginInstance {
    pub fn new(
        descriptor: PluginDescriptor,
        mut engine: Engine,
        parameters: Vec<Arc<Parameter>>,
    ) -> Result<Self, StateError> {
        if descriptor.id.is_empty() {
            return Err("plugin ID cannot be empty".into());
        }
        Preset::capture(descriptor.id, "validate", &parameters)?;
        for parameter in &parameters {
            let id = parameter_id(parameter.info().id);
            if id == u32::MAX {
                return Err("reserved parameter ID".into());
            }
            engine.bind_parameter(id, parameter.clone())?;
        }
        Ok(Self {
            descriptor,
            engine,
            engine64: None,
            precision_scratch: Vec::new(),
            parameters,
            active: false,
            project: None,
            project_factory: None,
            project_normalizer: None,
            sidechain: None,
            sidechain_input: Vec::new(),
        })
    }
    /// Opt in to an f64 processing chain. Hosts using f32 are converted only
    /// at the product boundary and use the same processor history.
    pub fn new_64(
        descriptor: PluginDescriptor,
        mut engine: Engine64,
        parameters: Vec<Arc<Parameter>>,
    ) -> Result<Self, StateError> {
        let mut instance = Self::new(descriptor, Engine::default(), parameters)?;
        for parameter in &instance.parameters {
            engine.bind_parameter(parameter_id(parameter.info().id), parameter.clone())?;
        }
        instance.engine64 = Some(engine);
        Ok(instance)
    }

    pub fn supports_f64(&self) -> bool {
        self.engine64.is_some()
    }

    pub fn process_automated64(
        &mut self,
        audio: &mut [f64],
        events: &[Event],
        automation: &[AutomationEvent],
    ) -> Result<(), Error> {
        if !self.active {
            audio.fill(0.);
            return Err(Error::NotPrepared);
        }
        if !self.descriptor.accepts_midi && !events.is_empty() {
            audio.fill(0.);
            return Err(Error::InvalidEvents);
        }
        let Some(engine) = &mut self.engine64 else {
            audio.fill(0.);
            return Err(Error::InvalidSpec);
        };
        engine.process_automated(audio, events, automation)
    }

    pub fn process64(&mut self, audio: &mut [f64], events: &[Event]) -> Result<(), Error> {
        self.process_automated64(audio, events, &[])
    }

    /// Append a stereo-in/stereo-out processor driven by a separate stereo key.
    /// Construct while stopped. The main Engine still owns parameters/automation.
    pub fn with_sidechain(
        mut self,
        processor: impl BusProcessor + 'static,
    ) -> Result<Self, StateError> {
        if self.active || self.descriptor.is_instrument || self.supports_f64() {
            return Err("sidechain requires an inactive f32 effect".into());
        }
        self.sidechain = Some(BusEngine::new(processor));
        Ok(self)
    }
    pub fn has_sidechain(&self) -> bool {
        self.sidechain.is_some()
    }
    pub fn process_with_sidechain(
        &mut self,
        audio: &mut [f32],
        sidechain: &[f32],
        events: &[Event],
        automation: &[AutomationEvent],
    ) -> Result<(), Error> {
        if self.sidechain.is_some()
            && (audio.len() != sidechain.len() || audio.len() > self.sidechain_input.len())
        {
            audio.fill(0.0);
            return Err(Error::InvalidBuffer);
        }
        self.process_automated(audio, events, automation)?;
        if let Some(bus) = &mut self.sidechain {
            self.sidechain_input[..audio.len()].copy_from_slice(audio);
            bus.process(&self.sidechain_input[..audio.len()], audio, sidechain, &[])?;
        }
        Ok(())
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn activate(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.active = false;
        if let Some(engine) = &mut self.engine64 {
            engine.prepare(spec)?;
            self.precision_scratch
                .resize(spec.channels * spec.max_block_frames, 0.);
        } else {
            self.engine.prepare(spec)?;
        }
        if let Some(bus) = &mut self.sidechain {
            if spec.channels != 2 {
                return Err(Error::InvalidSpec);
            }
            bus.prepare(BusSpec {
                sample_rate: spec.sample_rate,
                max_block_frames: spec.max_block_frames,
                layout: BusLayout {
                    input_channels: 2,
                    output_channels: 2,
                    sidechain_channels: 2,
                },
            })?;
            self.sidechain_input = vec![0.0; spec.max_block_frames * 2];
        }
        self.active = true;
        Ok(())
    }
    pub fn deactivate(&mut self) {
        self.active = false;
        self.reset();
    }
    pub fn process(&mut self, audio: &mut [f32], events: &[Event]) -> Result<(), Error> {
        self.process_automated(audio, events, &[])
    }
    pub fn with_project(
        mut self,
        project: ProjectState,
        factory: impl Fn(&ProjectState, &[Arc<Parameter>]) -> Result<Engine, StateError>
            + Send
            + Sync
            + 'static,
    ) -> Result<Self, StateError> {
        if self.supports_f64() {
            return Err("use preset state for f64 products".into());
        }
        project.validate(self.descriptor.id)?;
        self.project = Some(project);
        self.project_factory = Some(Arc::new(factory));
        Ok(self)
    }
    pub fn project_state(&self) -> Option<&ProjectState> {
        self.project.as_ref()
    }
    /// Control-thread migration before state reconstruction and parameter validation.
    pub fn with_project_normalizer(
        mut self,
        normalizer: impl Fn(&ProjectState) -> Result<ProjectState, StateError> + Send + Sync + 'static,
    ) -> Result<Self, StateError> {
        if self.project.is_none() || self.active {
            return Err("normalizer requires an inactive project product".into());
        }
        self.project_normalizer = Some(Arc::new(normalizer));
        Ok(self)
    }
    pub fn save_state(&self) -> Result<String, StateError> {
        let preset = Preset::capture(self.descriptor.id, "Host state", &self.parameters)?;
        if let Some(state) = &self.project {
            let mut state = state.clone();
            state.parameters = preset.parameters;
            state.to_json()
        } else {
            preset.to_json()
        }
    }
    /// Deactivate first; state restoration cannot run concurrently with audio.
    pub fn load_state(&mut self, json: &str) -> Result<(), StateError> {
        if self.active {
            return Err("deactivate before restoring state".into());
        }
        if let Ok(preset) = Preset::from_json(json) {
            return preset.restore(self.descriptor.id, &self.parameters);
        }
        let mut state = ProjectState::from_json(json)?;
        state.validate(self.descriptor.id)?;
        if let Some(normalize) = &self.project_normalizer {
            state = normalize(&state)?;
            state.validate(self.descriptor.id)?;
        }
        let factory = self
            .project_factory
            .as_ref()
            .ok_or("this product has no project reconstruction factory")?;
        let mut engine = factory(&state, &self.parameters)?;
        for p in &self.parameters {
            engine.bind_parameter(parameter_id(p.info().id), p.clone())?;
        }
        let mut values = std::collections::BTreeMap::new();
        for p in &self.parameters {
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
            if !value.is_finite() || !(info.min..=info.max).contains(&value) {
                return Err("project parameter out of range".into());
            }
            values.insert(info.id.to_owned(), value);
        }
        Preset {
            schema_version: 1,
            product_id: self.descriptor.id.into(),
            name: state.name.clone(),
            parameters: values,
        }
        .restore(self.descriptor.id, &self.parameters)?;
        self.engine = engine;
        for p in &self.parameters {
            p.set_modulation(0.0)?;
        }
        self.project = Some(state);
        Ok(())
    }
    pub fn latency_frames(&self) -> usize {
        self.engine64
            .as_ref()
            .map_or_else(|| self.engine.latency_frames(), Engine64::latency_frames)
            .saturating_add(self.sidechain.as_ref().map_or(0, BusEngine::latency_frames))
    }
    pub fn process_automated(
        &mut self,
        audio: &mut [f32],
        events: &[Event],
        automation: &[AutomationEvent],
    ) -> Result<(), Error> {
        if !self.active {
            audio.fill(0.0);
            return Err(Error::NotPrepared);
        }
        if !self.descriptor.accepts_midi && !events.is_empty() {
            audio.fill(0.0);
            return Err(Error::InvalidEvents);
        }
        if let Some(engine) = &mut self.engine64 {
            let Some(wide) = self.precision_scratch.get_mut(..audio.len()) else {
                audio.fill(0.);
                return Err(Error::BlockTooLarge);
            };
            for (out, &sample) in wide.iter_mut().zip(audio.iter()) {
                *out = f64::from(sample);
            }
            let result = engine.process_automated(wide, events, automation);
            for (out, &sample) in audio.iter_mut().zip(wide.iter()) {
                *out = sample as f32;
            }
            result
        } else {
            self.engine.process_automated(audio, events, automation)
        }
    }
    pub fn set_transport(&mut self, transport: Transport) -> Result<(), Error> {
        match &mut self.engine64 {
            Some(engine) => engine.set_transport(transport),
            None => self.engine.set_transport(transport),
        }
    }
    pub fn reset(&mut self) {
        if let Some(engine) = &mut self.engine64 {
            engine.reset();
        } else {
            self.engine.reset();
        }
        if let Some(bus) = &mut self.sidechain {
            bus.reset();
        }
    }
}
/// Stable FNV-1a IDs shared by the host adapter and engine. Constructor rejects
/// collisions within one product rather than silently aliasing parameters.
pub fn parameter_id(id: &str) -> u32 {
    soniclaw_core::stable_parameter_id(id)
}
/// Hosts the exact same plugin processor inside a standalone Soniclaw Engine.
/// Hardware I/O, recording and UI remain the enclosing application's concern.
pub struct HostedProcessor(pub PluginInstance);
impl AudioProcessor for HostedProcessor {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.0.activate(spec)
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        let _ = self.0.set_transport(context.transport);
        let _ = self.0.process(audio.samples_mut(), context.events);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_frames(&self) -> usize {
        self.0.latency_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_and_state_guard() {
        let mut plugin = PluginInstance::new(
            PluginDescriptor {
                id: "org.soniclaw.test",
                name: "Test",
                vendor: "Test",
                version: "0.1",
                is_instrument: false,
                accepts_midi: false,
            },
            Engine::default(),
            vec![],
        )
        .unwrap();
        let mut audio = [1.0; 4];
        assert!(plugin.process(&mut audio, &[]).is_err());
        assert_eq!(audio, [0.0; 4]);
        let state = plugin.save_state().unwrap();
        plugin
            .activate(ProcessSpec {
                sample_rate: 48000.0,
                channels: 2,
                max_block_frames: 64,
            })
            .unwrap();
        assert!(plugin.load_state(&state).is_err());
        plugin.deactivate();
        plugin.load_state(&state).unwrap();
    }
}
