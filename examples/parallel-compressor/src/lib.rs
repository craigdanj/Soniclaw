//! An actual CLAP parallel compressor with a host-automatable dry/wet control.
use soniclaw::{
    dsp::{
        composition::{MixingRule, Parallel, ProcessorChain},
        dynamics::{self, Dynamics},
    },
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
};
use std::sync::Arc;

struct Compressor {
    mix: Arc<Parameter>,
    parallel: Parallel<Dynamics>,
}

impl AudioProcessor for Compressor {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.parallel.set_mix(self.mix.get())?;
        self.parallel.prepare(spec)
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        let _ = self.parallel.set_mix(self.mix.modulated());
        self.parallel.process(buffer, context);
    }

    fn reset(&mut self) {
        self.parallel.reset();
    }

    fn latency_frames(&self) -> usize {
        self.parallel.latency_frames()
    }
}

pub fn create() -> PluginInstance {
    let mix = Arc::new(
        Parameter::new(ParameterInfo {
            id: "mix",
            name: "Wet Mix",
            min: 0.,
            max: 1.,
            default: 0.5,
            unit: "",
        })
        .expect("valid mix parameter"),
    );
    let wet = Dynamics::new(dynamics::Config {
        lookahead_ms: 5.,
        controls: dynamics::Controls {
            attack_ms: 0.5,
            ..Default::default()
        },
        ..Default::default()
    })
    .expect("valid compressor configuration");
    let mut chain = ProcessorChain::default();
    chain.push(Compressor {
        mix: mix.clone(),
        parallel: Parallel::new(wet, MixingRule::Linear, 0.5, 64).expect("valid mix"),
    });
    let mut engine = Engine::default();
    engine.push(chain);
    PluginInstance::new(
        PluginDescriptor {
            id: "org.soniclaw.parallel-compressor",
            name: "Soniclaw Parallel Compressor",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: false,
            accepts_midi: false,
        },
        engine,
        vec![mix],
    )
    .expect("valid plug-in definition")
}

static DESCRIPTOR: soniclaw_clap::ClapDescriptor = soniclaw_clap::descriptor(
    c"org.soniclaw.parallel-compressor",
    c"Soniclaw Parallel Compressor",
    c"Soniclaw",
    c"0.146.0",
    false,
);
soniclaw_clap::export_clap!([soniclaw_clap::Definition {
    descriptor: &DESCRIPTOR,
    create,
    instrument: false,
    tail_seconds: 0.
}]);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plug_in_reports_latency_and_zero_mix_is_aligned() {
        let mut plug_in = create();
        plug_in.parameters()[0].set(0.).unwrap();
        plug_in
            .activate(ProcessSpec {
                sample_rate: 48_000.,
                channels: 2,
                max_block_frames: 128,
            })
            .unwrap();
        assert_eq!(plug_in.latency_frames(), 240);
        let mut pcm = [0.; 512];
        pcm[0] = 1.;
        for block in pcm.as_chunks_mut::<256>().0 {
            plug_in.process(block, &[]).unwrap();
        }
        assert_eq!(pcm[480], 1.);
        assert_eq!(pcm.iter().filter(|sample| **sample != 0.).count(), 1);
    }
}
