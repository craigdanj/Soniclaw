//! A reusable typed gain/filter processor exported as a real CLAP effect.
use soniclaw::{
    params::*,
    plugin::{PluginDescriptor, PluginInstance},
    prelude::*,
};
parameter_set! {pub struct Controls{
 gain:FloatParam=FloatParam::new(Definition::new("master.gain","Gain",Kind::Float{min:0.,max:2.,default:0.5,curve:Curve::Linear}).group("Output")),
 cutoff:FloatParam=FloatParam::new(Definition::new("filter.cutoff","Cutoff",Kind::Float{min:20.,max:20000.,default:200.,curve:Curve::Logarithmic}).unit("Hz").group("Filter")),
 stages:IntParam=IntParam::new(Definition::new("filter.stages","Stages",Kind::Integer{min:1,max:4,default:2}).group("Filter")),
 bypass:BoolParam=BoolParam::new(Definition::new("effect.bypass","Bypass",Kind::Boolean{default:false})),
 mode:ChoiceParam=ChoiceParam::new(Definition::new("filter.mode","Mode",Kind::Choice{labels:vec!["Gain only".into(),"Low pass".into(),"High pass".into()],default:0}).group("Filter"))
}}
struct Effect {
    controls: Controls,
    rate: f32,
    channels: usize,
    states: [[f32; 64]; 4],
}
impl AudioProcessor for Effect {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.rate = s.sample_rate as f32;
        self.channels = s.channels;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.controls.bypass.get() {
            return;
        }
        let gain = self.controls.gain.get();
        let alpha = 1.
            - (-2. * std::f32::consts::PI * self.controls.cutoff.get().min(self.rate * 0.49)
                / self.rate)
                .exp();
        let mode = self.controls.mode.get();
        let stages = self.controls.stages.get() as usize;
        for frame in b.samples_mut().chunks_exact_mut(self.channels) {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let input = *sample;
                let mut low = input;
                if mode > 0 {
                    for state in self.states.iter_mut().take(stages) {
                        state[channel] += alpha * (low - state[channel]);
                        low = state[channel];
                    }
                }
                *sample = match mode {
                    1 => low,
                    2 => input - low,
                    _ => input,
                } * gain;
            }
        }
    }
    fn reset(&mut self) {
        self.states = [[0.; 64]; 4];
    }
}
pub fn create_with(controls: Controls) -> PluginInstance {
    let raw = controls.raw_parameters();
    let mut engine = Engine::default();
    engine.push(Effect {
        controls,
        rate: 48000.,
        channels: 2,
        states: [[0.; 64]; 4],
    });
    PluginInstance::new(
        PluginDescriptor {
            id: "org.soniclaw.typed-effect",
            name: "Soniclaw Typed Effect",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: false,
            accepts_midi: false,
        },
        engine,
        raw,
    )
    .unwrap()
}
pub fn create() -> PluginInstance {
    create_with(Controls::new().unwrap())
}
static DESCRIPTOR: soniclaw_clap::ClapDescriptor = soniclaw_clap::descriptor(
    c"org.soniclaw.typed-effect",
    c"Soniclaw Typed Effect",
    c"Soniclaw",
    c"0.146.0",
    false,
);
soniclaw_clap::export_clap!([soniclaw_clap::Definition {
    descriptor: &DESCRIPTOR,
    create,
    instrument: false,
    tail_seconds: 0.5
}]);
