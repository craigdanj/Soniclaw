//! Driver timestamp delivery clock. No callback allocation or synchronization with a mutex.
use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Instant,
};
pub struct InputClock {
    epoch: Instant,
    sequence: AtomicU64,
    wall: AtomicU64,
    sample: AtomicU64,
    rate: AtomicU32,
    lookahead: AtomicU64,
}
impl Default for InputClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            sequence: AtomicU64::new(0),
            wall: AtomicU64::new(0),
            sample: AtomicU64::new(0),
            rate: AtomicU32::new(0),
            lookahead: AtomicU64::new(0),
        }
    }
}
impl InputClock {
    pub(crate) fn anchor(&self, sample: u64, rate: u32, lookahead: usize) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        self.wall.store(
            self.epoch.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            Ordering::SeqCst,
        );
        self.sample.store(sample, Ordering::SeqCst);
        self.rate.store(rate, Ordering::SeqCst);
        self.lookahead.store(lookahead as u64, Ordering::SeqCst);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    /// A scheduling anchor including one maximum internal block of intentional lookahead.
    pub fn now(&self) -> Option<(u64, u32)> {
        for _ in 0..3 {
            let seq = self.sequence.load(Ordering::SeqCst);
            if seq & 1 != 0 {
                continue;
            }
            let wall = self.wall.load(Ordering::SeqCst);
            let sample = self.sample.load(Ordering::SeqCst);
            let rate = self.rate.load(Ordering::SeqCst);
            let delay = self.lookahead.load(Ordering::SeqCst);
            let now = self.epoch.elapsed().as_nanos().min(u64::MAX as u128) as u64;
            if self.sequence.load(Ordering::SeqCst) != seq {
                continue;
            }
            if rate == 0 || now.saturating_sub(wall) > 1_000_000_000 {
                return None;
            }
            let elapsed = ((now.saturating_sub(wall) as f64 * rate as f64) / 1e9).round() as u64;
            return Some((sample.saturating_add(elapsed).saturating_add(delay), rate));
        }
        None
    }
    pub fn lookahead_frames(&self) -> u64 {
        self.lookahead.load(Ordering::Acquire)
    }
}
