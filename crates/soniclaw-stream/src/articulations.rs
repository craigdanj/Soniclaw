//! Keyswitch configuration for a disk-paged instrument.
use crate::{DiskAsset, DiskZone};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneRule {
    /// MIDI keyswitch value. None is always eligible.
    pub keyswitch: Option<u8>,
    /// Optional linear release ramp duration, retained on already-started voices.
    pub release_ms: Option<f32>,
}
pub struct DiskInstrument {
    pub assets: Vec<DiskAsset>,
    pub zones: Vec<DiskZone>,
    pub rules: Vec<ZoneRule>,
    pub default_switch: Option<u8>,
}
impl DiskInstrument {
    pub fn open<const N: usize>(
        self,
        max_pcm_bytes: usize,
    ) -> Result<(crate::DiskHandle, crate::DiskSampler<N>), Box<dyn std::error::Error>> {
        let Self {
            assets,
            zones,
            rules,
            default_switch,
        } = self;
        let (handle, mut sampler) = crate::open_disk_sampler(assets, zones, max_pcm_bytes)?;
        sampler.set_articulations(rules, default_switch)?;
        Ok((handle, sampler))
    }
}
