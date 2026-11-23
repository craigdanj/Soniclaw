//! Prepared multichannel callback exchange. One device callback writes, then its engine reads.
use crate::Error;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[derive(Debug)]
pub struct ChannelBus {
    ready: AtomicBool,
    channels: usize,
    maximum: usize,
    data: Vec<AtomicU32>,
    frames: AtomicUsize,
    base: AtomicUsize,
    discontinuities: AtomicU64,
}
impl ChannelBus {
    pub fn new(channels: usize, maximum: usize) -> Result<Self, Error> {
        if !(1..=64).contains(&channels) || !(1..=8192).contains(&maximum) {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            ready: AtomicBool::new(true),
            channels,
            maximum,
            data: (0..channels * maximum).map(|_| AtomicU32::new(0)).collect(),
            frames: AtomicUsize::new(0),
            base: AtomicUsize::new(0),
            discontinuities: AtomicU64::new(0),
        })
    }
    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Release);
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn maximum_frames(&self) -> usize {
        self.maximum
    }
    /// Begin a callback publication, clearing visible frames without allocating.
    pub fn begin(&self) {
        self.frames.store(0, Ordering::Release);
        self.base.store(0, Ordering::Relaxed);
    }
    pub fn offset(&self) -> usize {
        self.base.load(Ordering::Relaxed)
    }
    /// Publish a contiguous subrange at the current outer-block origin.
    pub fn write_at(&self, offset: usize, pcm: &[f32]) -> Result<(), Error> {
        let first = self
            .offset()
            .checked_add(offset)
            .ok_or(Error::InvalidBuffer)?;
        if !pcm.len().is_multiple_of(self.channels) || pcm.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidBuffer);
        }
        let end = first
            .checked_add(pcm.len() / self.channels)
            .filter(|n| *n <= self.maximum)
            .ok_or(Error::InvalidBuffer)?;
        if first > self.frames.load(Ordering::Acquire) {
            return Err(Error::InvalidBuffer);
        }
        for (d, s) in self.data[first * self.channels..end * self.channels]
            .iter()
            .zip(pcm)
        {
            d.store(s.to_bits(), Ordering::Relaxed);
        }
        self.frames.fetch_max(end, Ordering::Release);
        Ok(())
    }
    /// Add a monitor overlay after publishing a range; one callback owns writes.
    pub fn add_sample(&self, frame: usize, channel: usize, value: f32) -> Result<(), Error> {
        let at = self
            .offset()
            .checked_add(frame)
            .filter(|f| *f < self.frames.load(Ordering::Acquire))
            .ok_or(Error::InvalidBuffer)?;
        if channel >= self.channels || !value.is_finite() {
            return Err(Error::InvalidBuffer);
        }
        let sample = &self.data[at * self.channels + channel];
        let value = f32::from_bits(sample.load(Ordering::Relaxed)) + value;
        if !value.is_finite() {
            return Err(Error::InvalidBuffer);
        }
        sample.store(value.to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn write(&self, pcm: &[f32]) -> Result<(), Error> {
        if !pcm.len().is_multiple_of(self.channels)
            || pcm.len() > self.data.len()
            || pcm.iter().any(|s| !s.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        self.frames.store(0, Ordering::Release);
        for (d, s) in self.data.iter().zip(pcm) {
            d.store(s.to_bits(), Ordering::Relaxed);
        }
        self.base.store(0, Ordering::Relaxed);
        self.frames
            .store(pcm.len() / self.channels, Ordering::Release);
        Ok(())
    }
    /// Callback sub-block origin; reset to zero after a wrapper subdivides the device block.
    pub fn set_offset(&self, frame: usize) -> Result<(), Error> {
        if frame > self.frames.load(Ordering::Acquire) {
            return Err(Error::InvalidBuffer);
        }
        self.base.store(frame, Ordering::Relaxed);
        Ok(())
    }
    pub fn sample(&self, frame: usize, channel: usize) -> f32 {
        let frame = frame.saturating_add(self.base.load(Ordering::Relaxed));
        if channel >= self.channels || frame >= self.frames.load(Ordering::Acquire) {
            return 0.;
        }
        f32::from_bits(self.data[frame * self.channels + channel].load(Ordering::Relaxed))
    }
    pub fn mark_discontinuity(&self) {
        self.discontinuities.fetch_add(1, Ordering::Relaxed);
    }
    pub fn discontinuities(&self) -> u64 {
        self.discontinuities.load(Ordering::Acquire)
    }
}
#[cfg(test)]
mod publication_tests {
    use super::*;
    #[test]
    fn subranges_overlays_and_clear_keep_outer_callback_coordinates() {
        let b = ChannelBus::new(4, 8).unwrap();
        b.begin();
        b.write_at(0, &[0.1; 8]).unwrap();
        b.set_offset(2).unwrap();
        b.write_at(0, &[0.2; 8]).unwrap();
        b.add_sample(0, 1, 0.3).unwrap();
        b.set_offset(0).unwrap();
        assert_eq!(b.sample(0, 1), 0.1);
        assert_eq!(b.sample(2, 1), 0.5);
        assert_eq!(b.sample(3, 1), 0.2);
        assert_eq!(b.sample(4, 1), 0.);
        b.begin();
        assert_eq!(b.sample(0, 1), 0.);
        assert!(b.write_at(1, &[0.; 4]).is_err());
        assert!(b.add_sample(0, 0, 1.).is_err());
        assert!(b.write_at(0, &[f32::NAN; 4]).is_err());
    }
}
