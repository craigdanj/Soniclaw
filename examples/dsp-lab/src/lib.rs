//! Complete-state demonstration factories for the DSP expansion.
use soniclaw::{
    plugin::{PluginDescriptor, PluginInstance},
    state::{Connection, ProcessorKind, ProjectState, RackNode},
};
use soniclaw_clap::{descriptor, Definition};
pub fn effect_state() -> ProjectState {
    let mut state = ProjectState::empty("org.soniclaw.dsp-lab.effect", "Soniclaw DSP Lab FX");
    let chain = [
        ("filter", ProcessorKind::StateVariable),
        ("transient", ProcessorKind::TransientShaper),
        ("rms", ProcessorKind::RmsCompressor),
        ("phaser", ProcessorKind::Phaser),
        ("flanger", ProcessorKind::Flanger),
        ("delay", ProcessorKind::MultiTapDelay),
        ("space", ProcessorKind::FdnReverb),
        ("spectral", ProcessorKind::SpectralGate),
        ("limiter", ProcessorKind::LookaheadLimiter),
        ("stereo", ProcessorKind::StereoWidth),
    ];
    for (i, (id, kind)) in chain.into_iter().enumerate() {
        state.nodes.push(RackNode {
            custom: None,
            voice_modulation: None,
            asset: None,
            id: id.into(),
            kind,
            settings: Default::default(),
            input_gain: if i == 0 { 1.0 } else { 0.0 },
        });
        if i > 0 {
            state.connections.push(Connection {
                from: chain[i - 1].0.into(),
                to: id.into(),
                gain: 1.0,
            });
        }
    }
    state.outputs.insert("stereo".into(), 1.0);
    state
}
pub fn wave_state() -> ProjectState {
    let mut state = ProjectState::empty("org.soniclaw.dsp-lab.wavetable", "Soniclaw Wavetable Lab");
    state.nodes.push(RackNode {
        custom: None,
        voice_modulation: None,
        asset: None,
        id: "osc".into(),
        kind: ProcessorKind::Wavetable,
        settings: Default::default(),
        input_gain: 0.0,
    });
    state.outputs.insert("osc".into(), 1.0);
    state
}
pub fn create_effect() -> PluginInstance {
    make(effect_state(), false)
}
pub fn create_wave() -> PluginInstance {
    make(wave_state(), true)
}
fn make(state: ProjectState, instrument: bool) -> PluginInstance {
    let (id, name) = if instrument {
        ("org.soniclaw.dsp-lab.wavetable", "Soniclaw Wavetable Lab")
    } else {
        ("org.soniclaw.dsp-lab.effect", "Soniclaw DSP Lab FX")
    };
    soniclaw::project::create_plugin(
        PluginDescriptor {
            id,
            name,
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: instrument,
            accepts_midi: instrument,
        },
        state,
        ".",
    )
    .expect("valid DSP project factory")
}
static FX: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.dsp-lab.effect",
    c"Soniclaw DSP Lab FX",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static WAVE: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.dsp-lab.wavetable",
    c"Soniclaw Wavetable Lab",
    c"Soniclaw",
    c"0.146.0",
    true,
);
soniclaw_clap::export_clap!([
    Definition {
        descriptor: &FX,
        create: create_effect,
        instrument: false,
        tail_seconds: 8.0
    },
    Definition {
        descriptor: &WAVE,
        create: create_wave,
        instrument: true,
        tail_seconds: 0.5
    }
]);
