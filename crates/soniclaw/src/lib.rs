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
pub mod catalog;
pub mod project;
pub mod sampling;
pub use soniclaw_registry as registry;
pub mod project_loading;
pub mod project_storage;
pub use soniclaw_bus as bus;
pub use soniclaw_clips as clips;
pub use soniclaw_core as core;
#[cfg(feature = "native")]
pub use soniclaw_device as device;
pub use soniclaw_diagnostics as diagnostics;
pub use soniclaw_dsp as dsp;
pub use soniclaw_engine as engine;
pub use soniclaw_graph as graph;
#[cfg(feature = "hosting")]
pub use soniclaw_host as host;
pub use soniclaw_io as io;
pub use soniclaw_midi as midi;
pub use soniclaw_params as params;
pub use soniclaw_plugin as plugin;
pub use soniclaw_record as record;
pub use soniclaw_session as session;
pub use soniclaw_state as state;
#[cfg(feature = "ui")]
pub use soniclaw_ui as ui;
pub use soniclaw_waveform as waveform;
pub mod prelude {
    pub use soniclaw_core::{
        AudioBuffer, AudioBuffer64, AudioProcessor, AudioProcessor64, AutomationEvent, Error,
        Event, MidiMessage, MpeZone, NoteExpression, NoteTarget, Parameter, ParameterInfo,
        ProcessContext, ProcessSpec, SmoothedValue, Transport,
    };
    pub use soniclaw_dsp::{
        Adsr, AdsrSettings, Delay, ExpressiveSynth, Gain, LowPass, Oscillator, PolySynth, Sample,
        Sampler, Waveform,
    };
    pub use soniclaw_engine::{Engine, Engine64};
    pub use soniclaw_graph::Graph64;
}

pub use soniclaw_sampler as sampler;

pub use soniclaw_stream as stream;

pub use soniclaw_automation as automation;

#[cfg(feature = "formats")]
pub use soniclaw_formats as formats;
/// Blocking bounded asset decode. WAV stays portable; optional formats adds native AIFF/FLAC.
pub fn read_audio_with_limit(
    path: impl AsRef<std::path::Path>,
    limit: usize,
) -> Result<io::AudioFile, io::FileError> {
    let path = path.as_ref();
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("wav"))
    {
        return io::read_wav_with_limit(path, limit);
    }
    #[cfg(feature = "formats")]
    {
        formats::FormatManager::native_default()
            .map_err(|e| e as io::FileError)?
            .read_all(path, limit)
            .map_err(|e| e as io::FileError)
    }
    #[cfg(not(feature = "formats"))]
    {
        io::read_wav_with_limit(path, limit)
    }
}

pub use soniclaw_transport as transport;

#[cfg(feature = "vst3-hosting")]
pub use soniclaw_vst3 as vst3;

#[cfg(feature = "vst3-hosting")]
pub mod hosted;

pub mod rendering;

pub use soniclaw_commands as commands;

pub use soniclaw_mixer as mixer;

pub mod authoring;

pub use soniclaw_tuning as tuning;

pub use soniclaw_kernels as kernels;

pub use soniclaw_quality as quality;

pub use soniclaw_studio as studio;

#[cfg(feature = "pipewire")]
pub use soniclaw_pipewire as pipewire;
