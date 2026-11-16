//! Format-independent plug-in lifecycle and stable parameter identity. The
//! sonora-clap crate wraps this ownership boundary with an exported CLAP ABI.
use sonora_core::*;
use sonora_engine::Engine;
use sonora_state::{Preset, StateError};
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
        })
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn activate(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.active = false;
        self.engine.prepare(spec)?;
        self.active = true;
        Ok(())
    }
    pub fn deactivate(&mut self) {
        self.active = false;
        self.engine.reset();
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
        self.engine.latency_frames()
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
    }
}
/// Stable FNV-1a IDs shared by the host adapter and engine. Constructor rejects
/// collisions within one product rather than silently aliasing parameters.
pub fn parameter_id(id: &str) -> u32 {
    id.bytes().fold(2166136261u32, |hash, byte| {
        (hash ^ byte as u32).wrapping_mul(16777619)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_and_state_guard() {
        let mut plugin = PluginInstance::new(
            PluginDescriptor {
                id: "org.sonora.test",
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
