//! Sample-accurate playback of grouped microphone takes.
use soniclaw_core::{validate_events, Error, Event, MidiMessage};
use std::sync::Arc;

/// One captured hit. Channels are mono stems aligned at file origin.
pub struct Take {
    pub microphones: Vec<Arc<[f32]>>,
}

/// Velocity bounds are inclusive. Zones for the same note may not overlap.
pub struct Zone {
    pub note: u8,
    pub velocities: [u8; 2],
    pub takes: Vec<Take>,
    pub choke_group: Option<u16>,
}

#[derive(Clone, Copy)]
struct Voice {
    zone: usize,
    take: usize,
    frame: usize,
    channel: u8,
    gain: f32,
    fade: Option<u8>,
    age: u64,
}

pub struct MultiMicSampler {
    zones: Vec<Zone>,
    round_robin: Vec<usize>,
    zones_by_note: Vec<[Option<usize>; 128]>,
    voices: Vec<Option<Voice>>,
    mic_count: usize,
    rate: u32,
    max_frames: usize,
    clock: u64,
}

impl MultiMicSampler {
    pub fn new(
        rate: u32,
        mic_count: usize,
        max_frames: usize,
        polyphony: usize,
        zones: Vec<Zone>,
    ) -> Result<Self, Error> {
        if !(8000..=192_000).contains(&rate)
            || !(1..=16).contains(&mic_count)
            || !(1..=64).contains(&polyphony)
            || !(1..=8192).contains(&max_frames)
            || !(1..=256).contains(&zones.len())
        {
            return Err(Error::InvalidSpec);
        }
        let mut zones_by_note = vec![[None; 128]; 128];
        let mut budget = 0usize;
        for (index, zone) in zones.iter().enumerate() {
            let [low, high] = zone.velocities;
            if zone.note > 127
                || low == 0
                || low > high
                || high > 127
                || !(1..=64).contains(&zone.takes.len())
            {
                return Err(Error::InvalidParameter);
            }
            for entry in
                &mut zones_by_note[usize::from(zone.note)][usize::from(low)..=usize::from(high)]
            {
                if entry.replace(index).is_some() {
                    return Err(Error::InvalidRouting);
                }
            }
            for take in &zone.takes {
                if take.microphones.len() != mic_count {
                    return Err(Error::InvalidBuffer);
                }
                let len = take.microphones[0].len();
                if !(1..=rate as usize * 180).contains(&len) {
                    return Err(Error::InvalidBuffer);
                }
                budget = budget
                    .checked_add(len * mic_count)
                    .filter(|&n| n <= 32 * 1024 * 1024)
                    .ok_or(Error::InvalidBuffer)?;
                if take
                    .microphones
                    .iter()
                    .any(|s| s.len() != len || s.iter().any(|v| !v.is_finite() || v.abs() > 8.))
                {
                    return Err(Error::InvalidBuffer);
                }
            }
        }
        Ok(Self {
            round_robin: vec![0; zones.len()],
            zones,
            zones_by_note,
            voices: vec![None; polyphony],
            mic_count,
            rate,
            max_frames,
            clock: 0,
        })
    }
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn microphone_count(&self) -> usize {
        self.mic_count
    }

    fn fire(&mut self, channel: u8, note: u8, velocity: u8) {
        let Some(zone_index) = self.zones_by_note[usize::from(note)][usize::from(velocity)] else {
            return;
        };
        let zone = &self.zones[zone_index];
        if let Some(group) = zone.choke_group {
            for voice in self.voices.iter_mut().flatten() {
                if voice.channel == channel && self.zones[voice.zone].choke_group == Some(group) {
                    voice.fade.get_or_insert(32);
                }
            }
        }
        let take = &mut self.round_robin[zone_index];
        let slot = self
            .voices
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.map_or(0, |v| v.age))
                    .expect("nonempty pool")
                    .0
            });
        self.clock = self.clock.saturating_add(1);
        self.voices[slot] = Some(Voice {
            zone: zone_index,
            take: *take,
            frame: 0,
            channel,
            gain: f32::from(velocity) / 127.,
            fade: None,
            age: self.clock,
        });
        *take = (*take + 1) % zone.takes.len();
    }

    /// Output order follows Take::microphones. Events are local to this block.
    pub fn process(
        &mut self,
        microphones: &mut [&mut [f32]],
        events: &[Event],
    ) -> Result<(), Error> {
        if microphones.len() != self.mic_count {
            return Err(Error::InvalidBuffer);
        }
        let frames = microphones[0].len();
        if frames > self.max_frames || microphones.iter().any(|mic| mic.len() != frames) {
            return Err(Error::InvalidBuffer);
        }
        validate_events(events, frames)?;
        for mic in microphones.iter_mut() {
            mic.fill(0.);
        }
        let mut events = events.iter().peekable();
        for frame in 0..frames {
            while events.peek().is_some_and(|e| e.frame == frame) {
                match events.next().expect("peeked event").message {
                    MidiMessage::NoteOn {
                        channel,
                        note,
                        velocity: velocity @ 1..=127,
                    } => self.fire(channel, note, velocity),
                    MidiMessage::AllNotesOff => {
                        for voice in self.voices.iter_mut().flatten() {
                            voice.fade.get_or_insert(32);
                        }
                    }
                    _ => {}
                }
            }
            for slot in &mut self.voices {
                let Some(voice) = slot else {
                    continue;
                };
                let take = &self.zones[voice.zone].takes[voice.take].microphones;
                let gain = voice.gain * voice.fade.map_or(1., |t| f32::from(t) / 32.);
                for (out, track) in microphones.iter_mut().zip(take) {
                    out[frame] += gain * track[voice.frame];
                }
                voice.frame += 1;
                if let Some(fade) = &mut voice.fade {
                    *fade -= 1;
                }
                if voice.frame == take[0].len() || voice.fade == Some(0) {
                    *slot = None;
                }
            }
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        self.voices.fill(None);
        self.round_robin.fill(0);
        self.clock = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_round_robin_and_delay_survive_split_callbacks() {
        let take = |x: f32| Take {
            microphones: vec![
                Arc::from([x, 0., 0.]),
                Arc::from([0., 2. * x, 0.]),
                Arc::from([0., 0., 3. * x]),
            ],
        };
        let zones = vec![Zone {
            note: 38,
            velocities: [1, 127],
            takes: vec![take(1.), take(0.5)],
            choke_group: None,
        }];
        let mut dsp = MultiMicSampler::new(48_000, 3, 8, 8, zones).unwrap();
        let e = |frame| Event {
            frame,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 38,
                velocity: 127,
            },
        };
        let mut l = [0.; 8];
        let mut m = [0.; 8];
        let mut r = [0.; 8];
        let a = allocation_counter::measure(|| {
            dsp.process(&mut [&mut l[..2], &mut m[..2], &mut r[..2]], &[e(1)])
                .unwrap();
            dsp.process(&mut [&mut l[2..], &mut m[2..], &mut r[2..]], &[e(2)])
                .unwrap();
        });
        assert_eq!(a.count_total, 0);
        assert_eq!(l, [0., 1., 0., 0., 0.5, 0., 0., 0.]);
        assert_eq!(m, [0., 0., 2., 0., 0., 1., 0., 0.]);
        assert_eq!(r, [0., 0., 0., 3., 0., 0., 1.5, 0.]);
    }
}
