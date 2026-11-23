//! Persisted absolute CC learning and prepared parameter bindings.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub channel: u8,
    pub controller: u8,
    pub parameter: String,
    pub range: [f32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LearnMap {
    pub mappings: Vec<Mapping>,
}
impl LearnMap {
    pub fn learn(&mut self, channel: u8, controller: u8, parameter: &str) -> Result<(), Error> {
        let mut candidate = self.clone();
        candidate.mappings.retain(|m| m.parameter != parameter);
        candidate.mappings.push(Mapping {
            channel,
            controller,
            parameter: parameter.into(),
            range: [0., 1.],
        });
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.mappings.len() > 128 {
            return Err(Error::InvalidParameter);
        }
        for (i, m) in self.mappings.iter().enumerate() {
            if m.channel > 15
                || m.controller > 127
                || m.parameter.is_empty()
                || m.parameter.len() > 256
                || m.range
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || m.range[0] >= m.range[1]
                || self.mappings[..i]
                    .iter()
                    .any(|p| p.parameter == m.parameter)
            {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
    pub fn compile(&self, parameters: &[Arc<Parameter>]) -> Result<LearnedControls, Error> {
        self.validate()?;
        let mut bindings = vec![];
        for m in &self.mappings {
            let p = parameters
                .iter()
                .find(|p| p.info().id == m.parameter)
                .ok_or(Error::InvalidParameter)?;
            bindings.push((m.clone(), p.clone()));
        }
        Ok(LearnedControls { bindings })
    }
}
pub struct LearnedControls {
    bindings: Vec<(Mapping, Arc<Parameter>)>,
}
impl LearnedControls {
    /// Apply when the embedding engine reaches the event's sample offset.
    pub fn apply(&self, message: MidiMessage) -> Result<usize, Error> {
        let MidiMessage::ControlChange {
            channel,
            controller,
            value,
        } = message
        else {
            return Ok(0);
        };
        if channel > 15 || controller > 127 || value > 127 {
            return Err(Error::InvalidEvents);
        }
        let mut count = 0;
        for (m, p) in &self.bindings {
            if m.channel == channel && m.controller == controller {
                let normalized = m.range[0] + value as f32 / 127.0 * (m.range[1] - m.range[0]);
                let info = p.info();
                p.set(info.min + normalized * (info.max - info.min))?;
                count += 1;
            }
        }
        Ok(count)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn learn_replaces_target_and_resolves_before_playback() {
        let p = Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: -60.,
                max: 6.,
                default: 0.,
                unit: "dB",
            })
            .unwrap(),
        );
        let mut map = LearnMap::default();
        map.learn(0, 7, "gain").unwrap();
        map.learn(1, 11, "gain").unwrap();
        assert_eq!(map.mappings.len(), 1);
        let c = map.compile(std::slice::from_ref(&p)).unwrap();
        assert_eq!(
            c.apply(MidiMessage::ControlChange {
                channel: 1,
                controller: 11,
                value: 127
            }),
            Ok(1)
        );
        assert_eq!(p.get(), 6.0);
        assert!(map.compile(&[]).is_err());
    }
}
