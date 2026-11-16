//! Format-independent plug-in lifecycle and stable parameter identity. The
//! soniclaw-clap crate wraps this ownership boundary with an exported CLAP ABI.
use soniclaw_bus::{BusEngine, BusLayout, BusProcessor, BusSpec};
use soniclaw_core::*;
use soniclaw_engine::Engine;
use soniclaw_state::{Preset, StateError};
use std::sync::Arc;

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
    parameters: Vec<Arc<Parameter>>,
    active: bool,
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
            parameters,
            active: false,
            sidechain: None,
            sidechain_input: Vec::new(),
        })
    }
    /// Append a stereo-in/stereo-out processor driven by a separate stereo key.
    /// Construct while stopped. The main Engine still owns parameters/automation.
    pub fn with_sidechain(
        mut self,
        processor: impl BusProcessor + 'static,
    ) -> Result<Self, StateError> {
        if self.active || self.descriptor.is_instrument {
            return Err("sidechain requires an inactive effect".into());
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
        self.engine.prepare(spec)?;
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
        self.engine.reset();
        if let Some(bus) = &mut self.sidechain {
            bus.reset();
        }
    }
    pub fn process(&mut self, audio: &mut [f32], events: &[Event]) -> Result<(), Error> {
        if !self.active {
            audio.fill(0.0);
            return Err(Error::NotPrepared);
        }
        if !self.descriptor.accepts_midi && !events.is_empty() {
            audio.fill(0.0);
            return Err(Error::InvalidEvents);
        }
        self.engine.process(audio, events)
    }
    pub fn save_state(&self) -> Result<String, StateError> {
        Preset::capture(self.descriptor.id, "Host state", &self.parameters)?.to_json()
    }
    /// Deactivate first; state restoration cannot run concurrently with audio.
    pub fn load_state(&mut self, json: &str) -> Result<(), StateError> {
        if self.active {
            return Err("deactivate before restoring state".into());
        }
        Preset::from_json(json)?.restore(self.descriptor.id, &self.parameters)
    }
    pub fn latency_frames(&self) -> usize {
        self.engine
            .latency_frames()
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
        self.engine.process_automated(audio, events, automation)
    }
    pub fn set_transport(&mut self, transport: Transport) -> Result<(), Error> {
        self.engine.set_transport(transport)
    }
    pub fn reset(&mut self) {
        self.engine.reset();
        if let Some(bus) = &mut self.sidechain {
            bus.reset();
        }
    }
}
/// Stable FNV-1a IDs shared by the host adapter and engine. Constructor rejects
/// collisions within one product rather than silently aliasing parameters.
pub fn parameter_id(id: &str) -> u32 {
    id.bytes().fold(2166136261u32, |hash, byte| {
        (hash ^ byte as u32).wrapping_mul(16777619)
    })
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
