use soniclaw::{
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
};
use soniclaw_clap::{descriptor, Definition};
use std::sync::Arc;
fn create(instrument: bool) -> PluginInstance {
    let gain = Arc::new(
        Parameter::new(ParameterInfo {
            id: "master.gain",
            name: "Gain",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: "",
        })
        .unwrap(),
    );
    let mut engine = Engine::default();
    if instrument {
        engine.push(soniclaw::dsp::ExpressiveSynth::<16>::default());
    }
    engine.push(Gain::new(gain.clone()));
    PluginInstance::new(
        PluginDescriptor {
            id: if instrument {
                "org.soniclaw.synth"
            } else {
                "org.soniclaw.gain"
            },
            name: if instrument {
                "Soniclaw Synth"
            } else {
                "Soniclaw Gain"
            },
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: instrument,
            accepts_midi: instrument,
        },
        engine,
        vec![gain],
    )
    .unwrap()
}
static GAIN: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.gain",
    c"Soniclaw Gain",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static SYNTH: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.synth",
    c"Soniclaw Synth",
    c"Soniclaw",
    c"0.146.0",
    true,
);
soniclaw_clap::export_clap!([
    Definition {
        descriptor: &GAIN,
        create: || create(false),
        instrument: false,
        tail_seconds: 0.0
    },
    Definition {
        descriptor: &SYNTH,
        create: || create(true),
        instrument: true,
        tail_seconds: 0.5
    },
]);
