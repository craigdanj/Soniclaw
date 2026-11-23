//! Named mono/stereo/multichannel buses and a prepared explicit channel patch bay.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Main,
    Auxiliary,
    Sidechain,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    pub id: String,
    pub name: String,
    pub channels: usize,
    pub role: Role,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub inputs: Vec<Bus>,
    pub outputs: Vec<Bus>,
}
impl Layout {
    pub fn validate(&self) -> Result<(), Error> {
        if self.outputs.is_empty() {
            return Err(Error::InvalidSpec);
        }
        for buses in [&self.inputs, &self.outputs] {
            if buses.len() > 8
                || buses.iter().any(|b| !(1..=64).contains(&b.channels))
                || buses.iter().map(|b| b.channels).sum::<usize>() > 64
            {
                return Err(Error::InvalidSpec);
            }
            for (i, b) in buses.iter().enumerate() {
                if b.id.is_empty()
                    || b.id.len() > 128
                    || b.name.len() > 256
                    || !(1..=64).contains(&b.channels)
                    || buses[..i].iter().any(|p| p.id == b.id)
                {
                    return Err(Error::InvalidSpec);
                }
            }
        }
        Ok(())
    }
    /// Exact stopped-time negotiation: no automatic mixing or silently discarded sidechains.
    pub fn negotiate(&self, supported: &[Layout]) -> Result<usize, Error> {
        self.validate()?;
        supported
            .iter()
            .position(|l| {
                l.validate().is_ok()
                    && l.inputs
                        .iter()
                        .map(|b| (&b.id, b.channels, b.role))
                        .eq(self.inputs.iter().map(|b| (&b.id, b.channels, b.role)))
                    && l.outputs
                        .iter()
                        .map(|b| (&b.id, b.channels, b.role))
                        .eq(self.outputs.iter().map(|b| (&b.id, b.channels, b.role)))
            })
            .ok_or(Error::InvalidSpec)
    }
    pub fn validate_buffers(
        &self,
        inputs: &[&[f32]],
        outputs: &[&mut [f32]],
        max_frames: usize,
    ) -> Result<usize, Error> {
        self.validate()?;
        if inputs.len() != self.inputs.len() || outputs.len() != self.outputs.len() {
            return Err(Error::InvalidBuffer);
        }
        let frames = outputs[0].len() / self.outputs[0].channels;
        if frames > max_frames {
            return Err(Error::BlockTooLarge);
        }
        if inputs
            .iter()
            .zip(&self.inputs)
            .any(|(b, l)| b.len() != frames * l.channels)
            || outputs
                .iter()
                .zip(&self.outputs)
                .any(|(b, l)| b.len() != frames * l.channels)
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(frames)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub input_bus: usize,
    pub input_channel: usize,
    pub output_bus: usize,
    pub output_channel: usize,
    pub gain: f32,
}
pub struct PatchBay {
    layout: Layout,
    routes: Vec<Route>,
    max_frames: usize,
}
impl PatchBay {
    pub fn new(layout: Layout, routes: Vec<Route>, max_frames: usize) -> Result<Self, Error> {
        layout.validate()?;
        if max_frames == 0 || max_frames > 8192 || routes.len() > 4096 {
            return Err(Error::InvalidSpec);
        }
        for r in &routes {
            if !r.gain.is_finite()
                || r.gain.abs() > 16.0
                || layout
                    .inputs
                    .get(r.input_bus)
                    .is_none_or(|b| r.input_channel >= b.channels)
                || layout
                    .outputs
                    .get(r.output_bus)
                    .is_none_or(|b| r.output_channel >= b.channels)
            {
                return Err(Error::InvalidRouting);
            }
        }
        Ok(Self {
            layout,
            routes,
            max_frames,
        })
    }
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) -> Result<(), Error> {
        let check = self
            .layout
            .validate_buffers(inputs, outputs, self.max_frames);
        for b in outputs.iter_mut() {
            b.fill(0.0);
        }
        let frames = check?;
        for r in &self.routes {
            let input = inputs[r.input_bus];
            let output = &mut outputs[r.output_bus];
            let ic = self.layout.inputs[r.input_bus].channels;
            let oc = self.layout.outputs[r.output_bus].channels;
            for frame in 0..frames {
                output[frame * oc + r.output_channel] +=
                    input[frame * ic + r.input_channel] * r.gain;
            }
        }
        Ok(())
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn layout() -> Layout {
        Layout {
            inputs: vec![
                Bus {
                    id: "main".into(),
                    name: "Mono".into(),
                    channels: 1,
                    role: Role::Main,
                },
                Bus {
                    id: "key".into(),
                    name: "Key".into(),
                    channels: 1,
                    role: Role::Sidechain,
                },
            ],
            outputs: vec![
                Bus {
                    id: "main".into(),
                    name: "Stereo".into(),
                    channels: 2,
                    role: Role::Main,
                },
                Bus {
                    id: "aux".into(),
                    name: "Aux".into(),
                    channels: 1,
                    role: Role::Auxiliary,
                },
            ],
        }
    }
    #[test]
    fn explicit_aux_routing_does_not_mix_sidechain() {
        let mut p = PatchBay::new(
            layout(),
            vec![
                Route {
                    input_bus: 0,
                    input_channel: 0,
                    output_bus: 0,
                    output_channel: 0,
                    gain: 1.0,
                },
                Route {
                    input_bus: 1,
                    input_channel: 0,
                    output_bus: 1,
                    output_channel: 0,
                    gain: 0.5,
                },
            ],
            8,
        )
        .unwrap();
        let (mut main, mut aux) = ([9.0; 4], [9.0; 2]);
        p.process(&[&[1., 2.], &[3., 4.]], &mut [&mut main, &mut aux])
            .unwrap();
        assert_eq!(main, [1., 0., 2., 0.]);
        assert_eq!(aux, [1.5, 2.]);
    }
    #[test]
    fn invalid_layouts_and_buffers_fail_silent() {
        let mut l = layout();
        assert_eq!(l.negotiate(&[l.clone()]), Ok(0));
        l.outputs[0].channels = 3;
        assert!(layout().negotiate(&[l]).is_err());
        let mut p = PatchBay::new(layout(), vec![], 8).unwrap();
        let mut out = [9.; 4];
        assert!(p.process(&[], &mut [&mut out]).is_err());
        assert_eq!(out, [0.; 4]);
    }
}
