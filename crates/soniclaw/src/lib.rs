//! Soniclaw's facade. Core/offline usage does not pull in a windowing or device stack.
//!
//! ```
//! use soniclaw::prelude::*;
//! let mut engine = Engine::default();
//! engine.push(PolySynth::<16>::default());
//! engine.prepare(ProcessSpec { sample_rate: 48000.0, max_block_frames: 256, channels: 2 })?;
//! let events = [Event { frame: 0, message: MidiMessage::NoteOn { channel: 0, note: 69, velocity: 100 } }];
//! let audio = soniclaw::engine::render(&mut engine, 4800, &events)?;
//! assert!(audio.iter().any(|x| x.abs() > 0.01));
//! # Ok::<(), soniclaw::core::Error>(())
//! ```
pub use soniclaw_bus as bus;
pub use soniclaw_core as core;
#[cfg(feature = "native")]
pub use soniclaw_device as device;
pub use soniclaw_dsp as dsp;
pub use soniclaw_engine as engine;
pub use soniclaw_graph as graph;
pub use soniclaw_io as io;
pub use soniclaw_midi as midi;
pub use soniclaw_plugin as plugin;
pub use soniclaw_record as record;
pub use soniclaw_session as session;
pub use soniclaw_state as state;
#[cfg(feature = "ui")]
pub use soniclaw_ui as ui;
pub mod prelude {
    pub use soniclaw_core::{
        AudioBuffer, AudioProcessor, AutomationEvent, Error, Event, MidiMessage, Parameter,
        ParameterInfo, ProcessContext, ProcessSpec, SmoothedValue, Transport,
    };
    pub use soniclaw_dsp::{
        Adsr, AdsrSettings, Delay, Gain, LowPass, Oscillator, PolySynth, Sample, Sampler, Waveform,
    };
    pub use soniclaw_engine::Engine;
}

pub use soniclaw_sampler as sampler;

pub use soniclaw_stream as stream;
