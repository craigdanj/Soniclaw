//! Shared reference processors. The same factories may be hosted in a standalone
//! application with `soniclaw::plugin::HostedProcessor` or exported as CLAP.
use soniclaw::{
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
    sampler::{MultiSampler, SampleBank, Zone},
};
use soniclaw_clap::{descriptor, Definition};
use std::sync::Arc;
pub fn parameter(
    id: &'static str,
    name: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> Arc<Parameter> {
    Arc::new(
        Parameter::new(ParameterInfo {
            id,
            name,
            min,
            max,
            default,
            unit,
        })
        .unwrap(),
    )
}
struct Reference {
    mode: Arc<Parameter>,
    gain: Gain,
    filter: LowPass,
}
impl AudioProcessor for Reference {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.gain.prepare(s)?;
        self.filter.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        let mode = self.mode.get().round() as u8;
        if mode == 2 {
            self.filter.process(b, c);
        }
        if mode != 0 {
            self.gain.process(b, c);
        }
    }
    fn reset(&mut self) {
        self.gain.reset();
        self.filter.reset();
    }
}
fn description(id: &'static str, name: &'static str, instrument: bool) -> PluginDescriptor {
    PluginDescriptor {
        id,
        name,
        vendor: "Soniclaw",
        version: env!("CARGO_PKG_VERSION"),
        is_instrument: instrument,
        accepts_midi: instrument,
    }
}
pub fn create_reference() -> PluginInstance {
    let gain = parameter("master.gain", "Gain", 0.0, 1.0, 0.5, "");
    let cutoff = parameter("filter.cutoff", "Cutoff", 20.0, 20000.0, 3000.0, " Hz");
    let mode = parameter("effect.mode", "Mode", 0.0, 2.0, 2.0, "");
    let mut engine = Engine::default();
    engine.push(Reference {
        mode: mode.clone(),
        gain: Gain::new(gain.clone()),
        filter: LowPass::new(cutoff.clone()),
    });
    PluginInstance::new(
        description("org.sonora.reference", "Soniclaw Gain + Filter", false),
        engine,
        vec![mode, gain, cutoff],
    )
    .unwrap()
}
pub fn create_ducker() -> PluginInstance {
    let gain = parameter("master.gain", "Gain", 0.0, 1.0, 1.0, "");
    let mut engine = Engine::default();
    engine.push(Gain::new(gain.clone()));
    PluginInstance::new(
        description("org.sonora.ducker", "Soniclaw Sidechain Ducker", false),
        engine,
        vec![gain],
    )
    .unwrap()
    .with_sidechain(soniclaw::bus::SidechainDucker::default())
    .unwrap()
}
pub fn create_drum_bank() -> Arc<SampleBank> {
    let mut samples = Vec::new();
    let mut zones = Vec::new();
    let mut random = 0x534f4e4fu32;
    for note in [36u8, 38, 42] {
        for (low, high, velocity_scale) in [(1, 79, 0.55), (80, 127, 1.0)] {
            for variant in 0..2 {
                let length = if note == 36 {
                    24000
                } else if note == 38 {
                    16000
                } else {
                    6500
                };
                let mut pcm = Vec::with_capacity(length);
                let mut phase = 0.0;
                for i in 0..length {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    let noise = random as f32 / u32::MAX as f32 * 2.0 - 1.0;
                    let t = i as f32 / 48000.0;
                    let value = if note == 36 {
                        let hz = 48.0 + 110.0 * (-t * 35.0).exp();
                        phase += std::f32::consts::TAU * hz / 48000.0;
                        phase.sin() * (-t * 12.0).exp() + noise * (-t * 140.0).exp() * 0.1
                    } else if note == 38 {
                        noise * (-t * (18.0 + variant as f32)).exp() * 0.6
                            + (std::f32::consts::TAU * 185.0 * t).sin() * (-t * 35.0).exp() * 0.3
                    } else {
                        noise * (-t * (48.0 + variant as f32 * 5.0)).exp() * 0.35
                    };
                    pcm.push(value * velocity_scale);
                }
                let sample = samples.len();
                samples.push(Sample::new(pcm, 1, 48000.0, note).unwrap());
                let mut z = Zone::single(sample, note);
                z.low_velocity = low;
                z.high_velocity = high;
                z.round_robin_group = note as u16;
                zones.push(z);
            }
        }
    }
    Arc::new(SampleBank::new(samples, zones, 4 * 1024 * 1024).unwrap())
}
pub fn create_drums() -> PluginInstance {
    let gain = parameter("master.gain", "Gain", 0.0, 1.0, 0.7, "");
    let mut e = Engine::default();
    e.push(MultiSampler::<64>::new(create_drum_bank()));
    e.push(Gain::new(gain.clone()));
    PluginInstance::new(
        description("org.sonora.drums", "Soniclaw Multisample Drums", true),
        e,
        vec![gain],
    )
    .unwrap()
}
pub fn create_effect_rack() -> PluginInstance {
    let gain = parameter("master.gain", "Gain", 0.0, 1.0, 0.5, "");
    let mut e = Engine::default();
    e.push(
        soniclaw::dsp::ParametricEq::new(vec![
            soniclaw::dsp::EqBand {
                shape: soniclaw::dsp::EqShape::HighPass,
                frequency: 60.0,
                q: 0.707,
                gain_db: 0.0,
            },
            soniclaw::dsp::EqBand {
                shape: soniclaw::dsp::EqShape::Peak,
                frequency: 1200.0,
                q: 0.8,
                gain_db: 2.0,
            },
        ])
        .unwrap(),
    );
    e.push(soniclaw::dsp::Compressor::new(Default::default()).unwrap());
    e.push(soniclaw::dsp::Chorus::new(0.7, 3.0, 0.15).unwrap());
    e.push(soniclaw::dsp::Reverb::new(0.8, 0.12).unwrap());
    e.push(Gain::new(gain.clone()));
    e.push(soniclaw::dsp::PeakLimiter::new(-1.0, 80.0).unwrap());
    PluginInstance::new(
        description("org.sonora.fx-rack", "Soniclaw Effects Rack", false),
        e,
        vec![gain],
    )
    .unwrap()
}
static REFERENCE: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.sonora.reference",
    c"Soniclaw Gain + Filter",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static DUCKER: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.sonora.ducker",
    c"Soniclaw Sidechain Ducker",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static DRUMS: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.sonora.drums",
    c"Soniclaw Multisample Drums",
    c"Soniclaw",
    c"0.146.0",
    true,
);
static RACK: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.sonora.fx-rack",
    c"Soniclaw Effects Rack",
    c"Soniclaw",
    c"0.146.0",
    false,
);
// A transparent topology makes accidental narrowing visible at the host boundary.
fn create_precision() -> PluginInstance {
    let mut engine = Engine64::default();
    engine.push(soniclaw::dsp::precision::Gain64::new(0.5).unwrap());
    PluginInstance::new_64(
        description(
            "org.soniclaw.precision-gain",
            "Soniclaw Precision Gain",
            false,
        ),
        engine,
        Vec::new(),
    )
    .expect("valid reference product")
}
static PRECISION: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.precision-gain",
    c"Soniclaw Precision Gain",
    c"Soniclaw",
    c"0.146.0",
    false,
);

soniclaw_clap::export_clap!([
    Definition {
        descriptor: &REFERENCE,
        create: create_reference,
        instrument: false,
        tail_seconds: 1.0
    },
    Definition {
        descriptor: &DUCKER,
        create: create_ducker,
        instrument: false,
        tail_seconds: 0.0
    },
    Definition {
        descriptor: &DRUMS,
        create: create_drums,
        instrument: true,
        tail_seconds: 1.0
    },
    Definition {
        descriptor: &RACK,
        create: create_effect_rack,
        instrument: false,
        tail_seconds: 4.0
    },
    Definition {
        descriptor: &PRECISION,
        create: create_precision,
        instrument: false,
        tail_seconds: 0.0
    },
]);
