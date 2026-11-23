//! Control-validated physical output mapping shared by native callbacks and virtual acceptance tests.
use soniclaw_core::{ChannelBus, Error};
#[derive(Clone, Copy, Debug)]
pub struct OutputMap {
    channels: usize,
    logical: usize,
    slots: [u8; 64],
}
impl OutputMap {
    pub fn new(channels: usize, main: [u16; 2], cues: &[[u16; 2]]) -> Result<Self, Error> {
        if !(2..=64).contains(&channels) || cues.len() > 4 {
            return Err(Error::InvalidRouting);
        }
        let mut slots = [255u8; 64];
        for (i, ch) in main.iter().chain(cues.iter().flatten()).enumerate() {
            if *ch as usize >= channels || slots[*ch as usize] != 255 {
                return Err(Error::InvalidRouting);
            }
            slots[*ch as usize] = i as u8;
        }
        Ok(Self {
            channels,
            logical: 2 + cues.len() * 2,
            slots,
        })
    }
    pub fn hardware_channels(&self) -> usize {
        self.channels
    }
    pub fn logical_channels(&self) -> usize {
        self.logical
    }
    /// Write one device frame, clearing unassigned channels. Returns the pre-clamp absolute peak.
    pub fn frame(
        &self,
        pair: [f32; 2],
        bank: Option<&ChannelBus>,
        frame: usize,
        destination: &mut [f32],
        enabled: bool,
    ) -> Result<f32, Error> {
        if destination.len() != self.channels
            || bank.is_some_and(|b| b.channels() != self.logical)
            || self.logical > 2 && bank.is_none()
        {
            destination.fill(0.);
            return Err(Error::InvalidBuffer);
        }
        let mut peak = 0f32;
        for (out, slot) in destination.iter_mut().zip(&self.slots) {
            let sample = match *slot {
                0 => pair[0],
                1 => pair[1],
                255 => 0.,
                n => bank.map_or(0., |b| b.sample(frame, n as usize)),
            };
            let sample = if sample.is_finite() { sample } else { 0. };
            peak = peak.max(sample.abs());
            *out = if enabled { sample.clamp(-1., 1.) } else { 0. };
        }
        Ok(peak)
    }
    pub fn render(
        &self,
        main: &[f32],
        bank: Option<&ChannelBus>,
        destination: &mut [f32],
        enabled: bool,
    ) -> Result<f32, Error> {
        destination.fill(0.);
        if !main.len().is_multiple_of(2) || destination.len() != main.len() / 2 * self.channels {
            return Err(Error::InvalidBuffer);
        }
        let mut peak = 0f32;
        for (i, (source, out)) in main
            .as_chunks::<2>()
            .0
            .iter()
            .zip(destination.chunks_exact_mut(self.channels))
            .enumerate()
        {
            peak = peak.max(self.frame(*source, bank, i, out, enabled)?);
        }
        Ok(peak)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_mapping_sanitizes_clamps_mutes_and_rejects_overlaps() {
        let map = OutputMap::new(6, [2, 3], &[[4, 5]]).unwrap();
        let bank = ChannelBus::new(4, 2).unwrap();
        bank.write(&[0., 0., 0.25, -0.5]).unwrap();
        let mut out = [9.; 6];
        let peak = map
            .frame([2., f32::NAN], Some(&bank), 0, &mut out, true)
            .unwrap();
        assert_eq!(peak, 2.);
        assert_eq!(out, [0., 0., 1., 0., 0.25, -0.5]);
        map.frame([1., 1.], Some(&bank), 0, &mut out, false)
            .unwrap();
        assert_eq!(out, [0.; 6]);
        assert!(OutputMap::new(6, [0, 1], &[[1, 2]]).is_err());
        assert!(OutputMap::new(6, [0, 6], &[]).is_err());
        assert!(map.frame([0.; 2], None, 0, &mut out, true).is_err());
        assert_eq!(out, [0.; 6]);
    }
}
