//! Prepared controller banks with soft takeover, encoder decoding and coalesced MIDI CC feedback.
use serde::{Deserialize, Serialize};
use soniclaw_core::{Error, MidiMessage, Parameter};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Mode {
    Absolute { pickup: bool, tolerance: f32 },
    TwosComplement,
    BinaryOffset,
    SignMagnitude,
    Toggle,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub channel: u8,
    pub controller: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub channel: u8,
    pub controller: u8,
    pub parameter: String,
    pub mode: Mode,
    pub step: f32,
    pub feedback: Option<Feedback>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank {
    pub name: String,
    pub mappings: Vec<Mapping>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema: u32,
    pub echo_window_ms: f32,
    pub banks: Vec<Bank>,
}
impl Profile {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || self.banks.is_empty()
            || self.banks.len() > 32
            || !self.echo_window_ms.is_finite()
            || !(0.0..=1000.).contains(&self.echo_window_ms)
        {
            return Err(Error::InvalidParameter);
        }
        for b in &self.banks {
            if b.name.is_empty() || b.name.len() > 256 || b.mappings.len() > 128 {
                return Err(Error::InvalidParameter);
            }
            let mut inputs = BTreeSet::new();
            let mut outputs = BTreeSet::new();
            for m in &b.mappings {
                if m.channel > 15
                    || m.controller > 127
                    || m.parameter.is_empty()
                    || m.parameter.len() > 256
                    || !m.step.is_finite()
                    || !(0.000001..=1.).contains(&m.step)
                    || !inputs.insert((m.channel, m.controller))
                {
                    return Err(Error::InvalidParameter);
                }
                if let Mode::Absolute { tolerance, .. } = m.mode {
                    if !tolerance.is_finite() || !(0.0..=0.25).contains(&tolerance) {
                        return Err(Error::InvalidParameter);
                    }
                }
                if let Some(f) = m.feedback {
                    if f.channel > 15
                        || f.controller > 127
                        || !outputs.insert((f.channel, f.controller))
                    {
                        return Err(Error::InvalidParameter);
                    }
                }
            }
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_json(s: &str) -> Result<Self, Error> {
        if s.len() > 1024 * 1024 {
            return Err(Error::InvalidParameter);
        }
        let p: Self = serde_json::from_str(s).map_err(|_| Error::InvalidParameter)?;
        p.validate()?;
        Ok(p)
    }
    pub fn compile(
        &self,
        parameters: &[Arc<Parameter>],
        sample_rate: f64,
    ) -> Result<Controller, Error> {
        self.validate()?;
        if !sample_rate.is_finite() || !(8000.0..=384000.).contains(&sample_rate) {
            return Err(Error::InvalidSpec);
        }
        let mut ids = BTreeSet::new();
        if parameters.iter().any(|p| !ids.insert(p.info().id)) {
            return Err(Error::InvalidParameter);
        }
        let mut banks = Vec::new();
        for b in &self.banks {
            let mut bindings = Vec::new();
            for m in &b.mappings {
                let p = parameters
                    .iter()
                    .find(|p| p.info().id == m.parameter)
                    .ok_or(Error::InvalidParameter)?
                    .clone();
                let target = p.normalized();
                bindings.push(Binding {
                    mapping: m.clone(),
                    parameter: p,
                    previous: None,
                    latched: false,
                    expected: target,
                    last_feedback: None,
                    pressed: false,
                });
            }
            banks.push(bindings);
        }
        Ok(Controller {
            banks,
            bank: 0,
            echo: [None; 2048],
            echo_frames: (sample_rate * self.echo_window_ms as f64 / 1000.).ceil() as u64,
            last_sample: None,
        })
    }
}
struct Binding {
    mapping: Mapping,
    parameter: Arc<Parameter>,
    previous: Option<f32>,
    latched: bool,
    expected: f32,
    last_feedback: Option<u8>,
    pressed: bool,
}
#[derive(Clone, Copy)]
struct Echo {
    value: u8,
    expires: u64,
}
pub struct Controller {
    banks: Vec<Vec<Binding>>,
    bank: usize,
    echo: [Option<Echo>; 2048],
    echo_frames: u64,
    last_sample: Option<u64>,
}
impl Controller {
    pub fn bank(&self) -> usize {
        self.bank
    }
    pub fn select_bank(&mut self, index: usize) -> Result<(), Error> {
        if index >= self.banks.len() {
            return Err(Error::InvalidParameter);
        }
        self.bank = index;
        for b in &mut self.banks[index] {
            b.previous = None;
            b.latched = false;
            b.expected = b.parameter.normalized();
            b.last_feedback = None;
            b.pressed = false;
        }
        Ok(())
    }
    /// Reset timestamp and echo history after a device/sample-clock discontinuity.
    pub fn reset_clock(&mut self) {
        self.last_sample = None;
        self.echo.fill(None);
        for b in self.banks.iter_mut().flatten() {
            b.previous = None;
            b.latched = false;
            b.last_feedback = None;
            b.pressed = false;
            b.expected = b.parameter.normalized();
        }
    }
    fn clock(&mut self, sample: u64) -> Result<(), Error> {
        if self.last_sample.is_some_and(|s| s > sample) {
            return Err(Error::InvalidEvents);
        }
        self.last_sample = Some(sample);
        Ok(())
    }
    /// Call at the MIDI event's sample offset on the processing owner. No allocations or driver calls.
    pub fn apply(&mut self, sample: u64, message: MidiMessage) -> Result<bool, Error> {
        let MidiMessage::ControlChange {
            channel,
            controller,
            value,
        } = message
        else {
            return Ok(false);
        };
        if channel > 15 || controller > 127 || value > 127 {
            return Err(Error::InvalidEvents);
        }
        self.clock(sample)?;
        let index = channel as usize * 128 + controller as usize;
        if self.echo_frames > 0
            && self.echo[index].is_some_and(|e| e.value == value && sample <= e.expires)
        {
            return Ok(false);
        }
        let Some(b) = self.banks[self.bank]
            .iter_mut()
            .find(|b| b.mapping.channel == channel && b.mapping.controller == controller)
        else {
            return Ok(false);
        };
        let target = b.parameter.normalized();
        let normalized = value as f32 / 127.;
        let next = match b.mapping.mode {
            Mode::Absolute { pickup, tolerance } => {
                if (target - b.expected).abs() > 1e-6 {
                    b.latched = false;
                }
                let crossed = b
                    .previous
                    .is_some_and(|p| (p - target) * (normalized - target) <= 0.);
                let accepted =
                    !pickup || b.latched || (normalized - target).abs() <= tolerance || crossed;
                b.previous = Some(normalized);
                if !accepted {
                    b.expected = target;
                    return Ok(false);
                }
                b.latched = true;
                normalized
            }
            Mode::Toggle => {
                let down = value >= 64;
                let rising = down && !b.pressed;
                b.pressed = down;
                if !rising {
                    return Ok(false);
                }
                if target >= 0.5 {
                    0.
                } else {
                    1.
                }
            }
            Mode::TwosComplement => {
                let delta = if value < 64 {
                    value as i16
                } else {
                    value as i16 - 128
                };
                if delta == 0 {
                    return Ok(false);
                }
                (target + delta as f32 * b.mapping.step).clamp(0., 1.)
            }
            Mode::BinaryOffset => {
                let delta = value as i16 - 64;
                if delta == 0 {
                    return Ok(false);
                }
                (target + delta as f32 * b.mapping.step).clamp(0., 1.)
            }
            Mode::SignMagnitude => {
                let delta = (value & 63) as i16 * if value & 64 == 0 { 1 } else { -1 };
                if delta == 0 {
                    return Ok(false);
                }
                (target + delta as f32 * b.mapping.step).clamp(0., 1.)
            }
        };
        b.parameter.set_normalized(next)?;
        b.expected = b.parameter.normalized();
        Ok(true)
    }
    /// Offer changed 7-bit values to a bounded nonblocking sink. Failed sends remain pending for retry.
    pub fn feedback(
        &mut self,
        sample: u64,
        mut send: impl FnMut([u8; 3]) -> bool,
    ) -> Result<usize, Error> {
        self.clock(sample)?;
        let mut count = 0;
        for b in &mut self.banks[self.bank] {
            let Some(f) = b.mapping.feedback else {
                continue;
            };
            let value = (b.parameter.normalized().clamp(0., 1.) * 127.).round() as u8;
            if b.last_feedback == Some(value) {
                continue;
            }
            let packet = [0xb0 | f.channel, f.controller, value];
            if send(packet) {
                b.last_feedback = Some(value);
                if self.echo_frames > 0 {
                    self.echo[f.channel as usize * 128 + f.controller as usize] = Some(Echo {
                        value,
                        expires: sample.saturating_add(self.echo_frames),
                    });
                }
                count += 1;
            }
        }
        Ok(count)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::ParameterInfo;
    fn parameter() -> Arc<Parameter> {
        Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.,
                max: 1.,
                default: 0.5,
                unit: "",
            })
            .unwrap(),
        )
    }
    fn profile(mode: Mode) -> Profile {
        Profile {
            schema: 1,
            echo_window_ms: 10.,
            banks: vec![Bank {
                name: "A".into(),
                mappings: vec![Mapping {
                    channel: 0,
                    controller: 7,
                    parameter: "gain".into(),
                    mode,
                    step: 0.01,
                    feedback: Some(Feedback {
                        channel: 0,
                        controller: 7,
                    }),
                }],
            }],
        }
    }
    fn cc(value: u8) -> MidiMessage {
        MidiMessage::ControlChange {
            channel: 0,
            controller: 7,
            value,
        }
    }
    #[test]
    fn pickup_external_change_and_bank_rearm() {
        let p = parameter();
        let mut profile = profile(Mode::Absolute {
            pickup: true,
            tolerance: 0.01,
        });
        profile.banks.push(profile.banks[0].clone());
        let mut c = profile.compile(std::slice::from_ref(&p), 48000.).unwrap();
        assert!(!c.apply(0, cc(0)).unwrap());
        assert!(!c.apply(1, cc(50)).unwrap());
        assert!(c.apply(2, cc(70)).unwrap());
        p.set(0.2).unwrap();
        assert!(!c.apply(3, cc(80)).unwrap());
        assert!(c.apply(4, cc(20)).unwrap());
        c.select_bank(1).unwrap();
        assert!(!c.apply(5, cc(127)).unwrap());
        assert!(c.apply(6, cc(0)).unwrap());
        assert!(c.apply(5, cc(127)).is_err());
        c.reset_clock();
        assert!(!c.apply(0, cc(127)).unwrap());
    }
    #[test]
    fn relative_modes_toggle_and_no_alloc() {
        for (mode, up, down) in [
            (Mode::TwosComplement, 1, 127),
            (Mode::BinaryOffset, 65, 63),
            (Mode::SignMagnitude, 1, 65),
        ] {
            let p = parameter();
            let mut c = profile(mode)
                .compile(std::slice::from_ref(&p), 48000.)
                .unwrap();
            let alloc = allocation_counter::measure(|| {
                assert!(c.apply(0, cc(up)).unwrap());
                assert!((p.get() - 0.51).abs() < 1e-6);
                assert!(c.apply(1, cc(down)).unwrap());
                assert!((p.get() - 0.5).abs() < 1e-6);
            });
            assert_eq!(alloc.count_total, 0);
        }
        let p = parameter();
        let mut c = profile(Mode::Toggle)
            .compile(std::slice::from_ref(&p), 48000.)
            .unwrap();
        assert!(c.apply(0, cc(127)).unwrap());
        assert_eq!(p.get(), 0.);
        assert!(!c.apply(1, cc(127)).unwrap());
        assert!(!c.apply(2, cc(0)).unwrap());
        assert!(c.apply(3, cc(127)).unwrap());
        assert_eq!(p.get(), 1.);
    }
    #[test]
    fn feedback_coalesces_retries_and_suppresses_only_sent_echoes() {
        let p = parameter();
        let profile = profile(Mode::Absolute {
            pickup: false,
            tolerance: 0.,
        });
        let profile = Profile::from_json(&profile.to_json().unwrap()).unwrap();
        let mut c = profile.compile(std::slice::from_ref(&p), 48000.).unwrap();
        assert_eq!(c.feedback(0, |_| false).unwrap(), 0);
        let mut packets = Vec::new();
        assert_eq!(
            c.feedback(1, |p| {
                packets.push(p);
                true
            })
            .unwrap(),
            1
        );
        assert_eq!(packets[0], [0xb0, 7, 64]);
        assert_eq!(c.feedback(2, |_| panic!("coalesced")).unwrap(), 0);
        assert!(!c.apply(3, cc(64)).unwrap());
        assert!(c.apply(482, cc(64)).unwrap());
        p.set(0.2).unwrap();
        assert_eq!(c.feedback(483, |_| true).unwrap(), 1);
        assert!(profile.compile(&[], 48000.).is_err());
    }
}
