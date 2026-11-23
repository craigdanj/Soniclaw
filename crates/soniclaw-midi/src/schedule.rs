//! Bounded allocation-free sample scheduler and explicit driver timestamp calibration.
use super::wire::Packet;
use soniclaw_core::Error;
use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};
#[derive(Clone, Copy, Debug)]
pub struct Scheduled {
    pub frame: usize,
    pub packet: Packet,
}
#[derive(Clone, Copy)]
struct Item {
    sample: u64,
    serial: u64,
    packet: Packet,
}
impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool {
        (self.sample, self.serial) == (o.sample, o.serial)
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.sample, self.serial).cmp(&(o.sample, o.serial))
    }
}
pub struct Scheduler {
    heap: BinaryHeap<Reverse<Item>>,
    capacity: usize,
    serial: u64,
    panic_channel: Option<u8>,
    pub overflow: u64,
    pub late: u64,
}
impl Scheduler {
    pub fn new(capacity: usize) -> Result<Self, Error> {
        if !(1..=4096).contains(&capacity) {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            heap: BinaryHeap::with_capacity(capacity),
            capacity,
            serial: 0,
            panic_channel: None,
            overflow: 0,
            late: 0,
        })
    }
    pub fn enqueue(&mut self, sample: u64, packet: Packet) -> Result<(), Error> {
        if self.heap.len() == self.capacity {
            self.overflow += 1;
            self.heap.clear();
            self.panic_channel = Some(0);
            return Err(Error::InvalidEvents);
        }
        let serial = self.serial;
        self.serial = self.serial.checked_add(1).ok_or(Error::InvalidEvents)?;
        self.heap.push(Reverse(Item {
            sample,
            serial,
            packet,
        }));
        Ok(())
    }
    pub fn panic(&mut self) {
        self.heap.clear();
        self.panic_channel = Some(0);
    }
    /// Exclusive end: an event at start+frames belongs to the next block. Late events clamp to frame zero.
    pub fn drain(
        &mut self,
        start: u64,
        frames: usize,
        output: &mut [Scheduled],
    ) -> Result<usize, Error> {
        if frames == 0 || frames > 8192 {
            return Err(Error::InvalidBuffer);
        }
        let end = start
            .checked_add(frames as u64)
            .ok_or(Error::InvalidBuffer)?;
        let mut n = 0;
        while n < output.len() {
            let Some(channel) = self.panic_channel else {
                break;
            };
            output[n] = Scheduled {
                frame: 0,
                packet: Packet::all_notes_off(channel)?,
            };
            n += 1;
            self.panic_channel = if channel == 15 {
                None
            } else {
                Some(channel + 1)
            };
        }
        if self.panic_channel.is_some() {
            return Ok(n);
        }
        while n < output.len() {
            let Some(Reverse(item)) = self.heap.peek() else {
                break;
            };
            if item.sample >= end {
                break;
            }
            let item = self.heap.pop().expect("peeked").0;
            if item.sample < start {
                self.late += 1;
            }
            output[n] = Scheduled {
                frame: item.sample.saturating_sub(start) as usize,
                packet: item.packet,
            };
            n += 1;
        }
        Ok(n)
    }
}
#[derive(Clone, Copy)]
pub struct TimestampMapper {
    micros: u64,
    sample: u64,
    rate: f64,
}
impl TimestampMapper {
    pub fn new(driver_micros: u64, audio_sample: u64, rate: f64) -> Result<Self, Error> {
        if !rate.is_finite() || !(1.0..=768000.0).contains(&rate) {
            return Err(Error::InvalidSpec);
        }
        Ok(Self {
            micros: driver_micros,
            sample: audio_sample,
            rate,
        })
    }
    pub fn sample(&self, driver_micros: u64) -> Result<u64, Error> {
        let delta = driver_micros as i128 - self.micros as i128;
        if delta.abs() > 86_400_000_000 {
            return Err(Error::InvalidEvents);
        }
        let offset = (delta as f64 * self.rate / 1_000_000.0).round() as i128;
        u64::try_from(self.sample as i128 + offset).map_err(|_| Error::InvalidEvents)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_stability_lateness_and_panic() {
        let p = Packet::new(&[0x90, 60, 100]).unwrap();
        let mut q = Scheduler::new(2).unwrap();
        q.enqueue(4, p).unwrap();
        q.enqueue(8, p).unwrap();
        let mut out = [Scheduled {
            frame: 0,
            packet: p,
        }; 16];
        assert_eq!(q.drain(0, 8, &mut out), Ok(1));
        assert_eq!(out[0].frame, 4);
        assert_eq!(q.drain(9, 4, &mut out), Ok(1));
        assert_eq!(out[0].frame, 0);
        assert_eq!(q.late, 1);
        q.enqueue(100, p).unwrap();
        q.enqueue(101, p).unwrap();
        assert!(q.enqueue(102, p).is_err());
        assert_eq!(q.drain(0, 8, &mut out), Ok(16));
        assert_eq!(out[15].packet.bytes(), &[0xBF, 123, 0]);
    }
    #[test]
    fn driver_anchor_is_explicit_and_bounded() {
        let m = TimestampMapper::new(1_000_000, 48000, 48000.0).unwrap();
        assert_eq!(m.sample(1_005_000), Ok(48240));
        assert!(m.sample(u64::MAX).is_err());
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    #[test]
    fn prepared_scheduler_performs_no_heap_operations() {
        let packet = Packet::new(&[0x90, 60, 100]).unwrap();
        let mut scheduler = Scheduler::new(64).unwrap();
        let mut output = [Scheduled { frame: 0, packet }; 64];
        let count = allocation_counter::measure(|| {
            for sample in 0..64 {
                scheduler.enqueue(sample, packet).unwrap();
            }
            assert_eq!(scheduler.drain(0, 64, &mut output), Ok(64));
        });
        assert_eq!(count.count_total, 0);
    }
}
