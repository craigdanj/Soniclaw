//! Double-precision processing contracts for applications that own their f64 buffers.
use crate::{Error, ProcessContext, ProcessSpec};

/// Interleaved, borrowed double-precision audio.
pub struct AudioBuffer64<'a> {
    data: &'a mut [f64],
    channels: usize,
}
impl<'a> AudioBuffer64<'a> {
    pub fn new(data: &'a mut [f64], channels: usize) -> Result<Self, Error> {
        if channels == 0 || !data.len().is_multiple_of(channels) {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self { data, channels })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
    pub fn samples(&self) -> &[f64] {
        self.data
    }
    pub fn samples_mut(&mut self) -> &mut [f64] {
        self.data
    }
    pub fn clear(&mut self) {
        self.data.fill(0.);
    }
}

/// Configure and allocate in prepare. Process one block without allocating, locking or I/O.
pub trait AudioProcessor64: Send {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error>;
    fn process(&mut self, buffer: &mut AudioBuffer64<'_>, context: &ProcessContext<'_>);
    fn reset(&mut self);
    fn latency_frames(&self) -> usize {
        0
    }
}
