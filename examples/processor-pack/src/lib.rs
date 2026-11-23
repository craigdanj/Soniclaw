//! An ordinary Rust crate that adds a processor without changing Soniclaw internals.
use soniclaw_core::*;
use soniclaw_registry::*;
use soniclaw_state::StateError;
use std::sync::Arc;
pub const TAPE_ID: &str = "org.soniclaw.example.tape";
pub fn register(registry: &mut ProcessorRegistry) -> Result<(), StateError> {
    let registration = Registration::new(
        ProcessorMetadata {
            id: TAPE_ID,
            name: "Tape Colour",
            category: "Distortion",
            version: 2,
            instrument: false,
        },
        vec![
            ParameterSpec {
                key: "drive",
                name: "Drive",
                min: 1.0,
                max: 12.0,
                default: 2.0,
                unit: "x",
            },
            ParameterSpec {
                key: "tone",
                name: "Tone",
                min: 100.0,
                max: 20000.0,
                default: 12000.0,
                unit: "Hz",
            },
            ParameterSpec {
                key: "mix",
                name: "Mix",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                unit: "",
            },
            ParameterSpec {
                key: "output",
                name: "Output",
                min: 0.0,
                max: 2.0,
                default: 1.0,
                unit: "x",
            },
        ],
        |ctx| {
            let value = &ctx.node.custom.as_ref().ok_or("missing tape state")?.data;
            let bias = if value.is_null() {
                0.0
            } else {
                let o = value.as_object().ok_or("tape data must be an object")?;
                if o.keys().any(|k| k != "asymmetry") {
                    return Err("unknown tape configuration".into());
                }
                o.get("asymmetry")
                    .map(|v| v.as_f64().ok_or("invalid tape asymmetry"))
                    .transpose()?
                    .unwrap_or(0.0) as f32
            };
            if !bias.is_finite() || !(-0.5..=0.5).contains(&bias) {
                return Err("tape asymmetry out of range".into());
            }
            Ok(Box::new(Tape {
                parameters: ctx.parameters.to_vec(),
                history: [0.0; 64],
                controls: [2.0, 12000.0, 0.5, 1.0],
                sample_rate: 48000.0,
                pole: 0.0,
                bias,
                prepared: false,
            }))
        },
    )
    .with_migration(|version, data| {
        if version != 1 {
            return Err("unsupported tape state version".into());
        }
        if data.is_null() {
            return Ok(serde_json::Value::Null);
        }
        let o = data.as_object().ok_or("invalid legacy tape data")?;
        if o.keys().any(|k| k != "bias") {
            return Err("invalid legacy tape data".into());
        }
        Ok(serde_json::json!({"asymmetry":o.get("bias").cloned().unwrap_or(0.0.into())}))
    });
    #[cfg(feature = "ui")]
    let registration = registration.with_editor(|ui, params| {
        ui.heading("Tape Colour");
        ui.label("Example processor supplied by a separate Rust crate");
        let mut commit = false;
        for p in params {
            let i = p.info();
            let mut v = p.get();
            let response = ui.add(
                egui::Slider::new(&mut v, i.min..=i.max)
                    .text(i.name)
                    .suffix(i.unit)
                    .logarithmic(i.id.ends_with("tone")),
            );
            if response.changed() {
                let _ = p.set(v);
            }
            commit |= response.drag_stopped() || (response.changed() && !response.dragged());
        }
        commit
    });
    registry.register(registration)
}
struct Tape {
    parameters: Vec<Arc<Parameter>>,
    history: [f32; 64],
    controls: [f32; 4],
    sample_rate: f32,
    pole: f32,
    bias: f32,
    prepared: bool,
}
impl AudioProcessor for Tape {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.prepared = false;
        s.validate()?;
        self.sample_rate = s.sample_rate as f32;
        self.pole = 1.0 - (-1.0 / (self.sample_rate * 0.005)).exp();
        for (v, p) in self.controls.iter_mut().zip(&self.parameters) {
            *v = p.modulated();
        }
        self.history.fill(0.0);
        self.prepared = true;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if !self.prepared {
            b.clear();
            return;
        }
        let ch = b.channels();
        for frame in b.samples_mut().chunks_exact_mut(ch) {
            for (v, p) in self.controls.iter_mut().zip(&self.parameters) {
                *v += (p.modulated() - *v) * self.pole;
            }
            let [drive, tone, mix, gain] = self.controls;
            let a = 1.0
                - (-std::f32::consts::TAU * tone.min(self.sample_rate * 0.45) / self.sample_rate)
                    .exp();
            let offset = (self.bias * drive).tanh();
            for (channel, x) in frame.iter_mut().enumerate() {
                let dry = if x.is_finite() { *x } else { 0.0 };
                let wet = ((dry + self.bias) * drive).tanh() - offset;
                self.history[channel] += (wet - self.history[channel]) * a;
                *x = (dry + (self.history[channel] - dry) * mix) * gain;
            }
        }
    }
    fn reset(&mut self) {
        self.history.fill(0.0);
    }
}
