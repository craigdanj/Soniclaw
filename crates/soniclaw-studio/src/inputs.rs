//! Prepared physical input routes, independent of the stereo mixing engine.
use crate::*;
#[derive(Clone)]
pub struct Setup {
    pub bus: Arc<ChannelBus>,
    pub routes: BTreeMap<String, Vec<u16>>,
}
impl Setup {
    pub fn validate(&self, d: &Document) -> Result<(), Error> {
        if self.bus.maximum_frames() < d.max_block_frames
            || self.routes.iter().any(|(id, chs)| {
                !d.tracks.iter().any(|t| &t.id == id)
                    || !(1..=2).contains(&chs.len())
                    || chs.iter().any(|&c| c as usize >= self.bus.channels())
            })
        {
            return Err(Error::InvalidRouting);
        }
        Ok(())
    }
}
impl Document {
    pub fn compile_inputs_at(
        &self,
        assets: &Assets,
        disk: &DiskAssets,
        factory: &mut impl Factory,
        start: u64,
        inputs: &Setup,
    ) -> Result<PreparedSession, Error> {
        inputs.validate(self)?;
        if start > 1 << 48 {
            return Err(Error::InvalidParameter);
        }
        self.compile_mode(assets, disk, factory, false, start, Some(inputs))
    }
}
