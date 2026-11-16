//! Shared contracts. No third-party dependencies; processing uses borrowed storage.
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidSpec,
    InvalidBuffer,
    NotPrepared,
    BlockTooLarge,
    InvalidEvents,
    InvalidParameter,
    InvalidRouting,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy)]
pub struct ProcessSpec {
    pub sample_rate: f64,
    pub max_block_frames: usize,
    pub channels: usize,
}
impl ProcessSpec {
    pub fn validate(self) -> Result<Self, Error> {
        if !self.sample_rate.is_finite()
            || self.sample_rate < 1.0
            || self.sample_rate > 768000.0
            || self.max_block_frames == 0
            || self.channels == 0
            || self.channels > 64
            || self.max_block_frames.checked_mul(self.channels).is_none()
        {
            return Err(Error::InvalidSpec);
        }
        Ok(self)
    }
}

/// Interleaved, non-owning audio. Construction checks complete frames.
pub struct AudioBuffer<'a> {
    data: &'a mut [f32],
    channels: usize,
}
impl<'a> AudioBuffer<'a> {
    pub fn new(data: &'a mut [f32], channels: usize) -> Result<Self, Error> {
        if channels == 0 || !data.len().is_multiple_of(channels) {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self { data, channels })
    }
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn samples(&self) -> &[f32] {
        self.data
    }
    pub fn samples_mut(&mut self) -> &mut [f32] {
        self.data
    }
    pub fn clear(&mut self) {
        self.data.fill(0.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    AllNotesOff,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    /// Sample offset relative to the beginning of this block, strictly < frames.
    pub frame: usize,
    pub message: MidiMessage,
}
/// Numeric IDs are assigned by the application and must remain stable in a product.
#[derive(Debug, Clone, Copy)]
pub struct AutomationEvent {
    pub frame: usize,
    pub parameter: u32,
    pub value: f32,
}
pub fn validate_events(events: &[Event], frames: usize) -> Result<(), Error> {
    let mut last = 0;
    for e in events {
        if e.frame >= frames || e.frame < last {
            return Err(Error::InvalidEvents);
        }
        last = e.frame;
    }
    Ok(())
}
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    pub sample_position: u64,
    pub tempo: f64,
    pub playing: bool,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            sample_position: 0,
            tempo: 120.0,
            playing: true,
        }
    }
}
pub struct ProcessContext<'a> {
    pub events: &'a [Event],
    pub transport: Transport,
}

/// Allocate/configure in `prepare`; never allocate, lock, log or do I/O in `process`.
/// Effects process in place, sources replace their input. Implementations must
/// accept any block length up to the prepared maximum, including zero frames.
pub trait AudioProcessor: Send {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error>;
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>);
    fn reset(&mut self);
    fn latency_frames(&self) -> usize {
        0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParameterInfo {
    /// Stable identifier: persist this, never the display name or slider position.
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}
/// A single atomic scalar. Independent parameters are not a coherent snapshot.
pub struct Parameter {
    info: ParameterInfo,
    value: AtomicU32,
}
impl Parameter {
    pub fn new(info: ParameterInfo) -> Result<Self, Error> {
        if info.id.is_empty()
            || !info.min.is_finite()
            || !info.max.is_finite()
            || info.min >= info.max
            || !info.default.is_finite()
            || !(info.min..=info.max).contains(&info.default)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            info,
            value: AtomicU32::new(info.default.to_bits()),
        })
    }
    pub fn info(&self) -> ParameterInfo {
        self.info
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.value.load(Ordering::Relaxed))
    }
    pub fn set(&self, value: f32) -> Result<(), Error> {
        if !value.is_finite() {
            return Err(Error::InvalidParameter);
        }
        self.value.store(
            value.clamp(self.info.min, self.info.max).to_bits(),
            Ordering::Relaxed,
        );
        Ok(())
    }
    pub fn normalized(&self) -> f32 {
        (self.get() - self.info.min) / (self.info.max - self.info.min)
    }
    pub fn set_normalized(&self, n: f32) -> Result<(), Error> {
        if !n.is_finite() {
            return Err(Error::InvalidParameter);
        }
        self.set(self.info.min + n.clamp(0.0, 1.0) * (self.info.max - self.info.min))
    }
}

/// Linear ramp; only restarting on target changes avoids block-size dependence.
pub struct SmoothedValue {
    current: f32,
    target: f32,
    step: f32,
    remaining: usize,
    ramp_frames: usize,
}
impl SmoothedValue {
    pub fn new(value: f32, ramp_frames: usize) -> Self {
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            ramp_frames,
        }
    }
    pub fn set_target(&mut self, target: f32) {
        if !target.is_finite() || target == self.target {
            return;
        }
        self.target = target;
        self.remaining = self.ramp_frames;
        if self.remaining == 0 {
            self.current = target;
        } else {
            self.step = (target - self.current) / self.remaining as f32;
        }
    }
    pub fn next_value(&mut self) -> f32 {
        if self.remaining > 0 {
            self.current += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
            }
        }
        self.current
    }
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_partial_frame_and_bad_spec() {
        assert!(AudioBuffer::new(&mut [0.0; 3], 2).is_err());
        assert!(ProcessSpec {
            sample_rate: f64::NAN,
            channels: 2,
            max_block_frames: 64
        }
        .validate()
        .is_err());
    }
    #[test]
    fn parameters_reject_nan_and_clamp() {
        let p = Parameter::new(ParameterInfo {
            id: "gain",
            name: "Gain",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: "",
        })
        .unwrap();
        p.set(2.0).unwrap();
        assert_eq!(p.get(), 1.0);
        assert!(p.set(f32::NAN).is_err());
        assert_eq!(p.get(), 1.0);
    }
    #[test]
    fn ramp_reaches_exact_target_without_restarting() {
        let mut s = SmoothedValue::new(0.0, 4);
        s.set_target(1.0);
        assert_eq!(s.next_value(), 0.25);
        s.set_target(1.0);
        assert_eq!(s.next_value(), 0.5);
        s.next_value();
        assert_eq!(s.next_value(), 1.0);
    }
    #[test]
    fn events_must_fit_and_be_ordered() {
        let e = |frame| Event {
            frame,
            message: MidiMessage::AllNotesOff,
        };
        assert!(validate_events(&[e(2), e(1)], 4).is_err());
        assert!(validate_events(&[e(4)], 4).is_err());
        assert!(validate_events(&[e(0), e(0), e(3)], 4).is_ok());
    }
}
