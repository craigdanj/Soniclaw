//! Prepared DSP expansion. Constructors and prepare may allocate; rendering does not.
use soniclaw_core::*;
pub mod analysis;
pub mod creative;
pub mod crossover;
pub mod dynamics;
pub mod filters;
pub mod reverb;
pub mod spectral;
pub mod wavetable;
pub(crate) trait LiveDsp: AudioProcessor {
    fn update(&mut self, values: &[f32]);
}
pub(crate) fn spec(s: ProcessSpec) -> Result<ProcessSpec, Error> {
    s.validate()?;
    if !(8000.0..=192000.0).contains(&s.sample_rate)
        || s.sample_rate * s.channels as f64 > 8_000_000.0
    {
        return Err(Error::InvalidSpec);
    }
    Ok(s)
}
pub(crate) fn clean(x: f32) -> f32 {
    if x.is_finite() && x.abs() > 1e-25 {
        x
    } else {
        0.0
    }
}
pub(crate) fn pole(ms: f32, sr: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sr).max(1.0)).exp()
}
#[cfg(test)]
pub(crate) fn context() -> ProcessContext<'static> {
    ProcessContext {
        events: &[],
        transport: Transport::default(),
    }
}
