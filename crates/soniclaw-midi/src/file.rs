//! Bounded PPQN SMF type 0/1 import and type 1 export; not a lossless editor.
use soniclaw_core::{Error, MidiMessage, NoteTarget};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoChange {
    pub tick: u64,
    pub micros_per_quarter: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipEvent {
    pub tick: u64,
    pub message: MidiMessage,
}
#[derive(Clone, Debug)]
pub struct MidiClip {
    pub ppqn: u16,
    pub tempos: Vec<TempoChange>,
    pub events: Vec<ClipEvent>,
}
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or(Error::InvalidEvents)?;
        let x = self.data.get(self.at..end).ok_or(Error::InvalidEvents)?;
        self.at = end;
        Ok(x)
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("two")))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four")))
    }
    fn vlq(&mut self) -> Result<u32, Error> {
        let mut value = 0;
        for _ in 0..4 {
            let byte = self.byte()?;
            value = (value << 7) | ((byte & 127) as u32);
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(Error::InvalidEvents)
    }
}
fn vlq(value: u64, out: &mut Vec<u8>) -> Result<(), Error> {
    if value > 0x0fff_ffff {
        return Err(Error::InvalidEvents);
    }
    let mut n = value as u32;
    let mut bytes = [0; 4];
    let mut i = 3;
    bytes[i] = (n & 127) as u8;
    n >>= 7;
    while n > 0 {
        i -= 1;
        bytes[i] = ((n & 127) as u8) | 128;
        n >>= 7;
    }
    out.extend_from_slice(&bytes[i..]);
    Ok(())
}
impl MidiClip {
    pub fn read(data: &[u8]) -> Result<Self, Error> {
        if data.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidEvents);
        }
        let mut r = Reader { data, at: 0 };
        if r.take(4)? != b"MThd" || r.u32()? != 6 {
            return Err(Error::InvalidEvents);
        }
        let format = r.u16()?;
        let tracks = r.u16()?;
        let ppqn = r.u16()?;
        if format > 1
            || tracks == 0
            || tracks > 64
            || (format == 0 && tracks != 1)
            || ppqn == 0
            || ppqn & 0x8000 != 0
        {
            return Err(Error::InvalidEvents);
        }
        let mut tempos = vec![];
        let mut events = vec![];
        for _ in 0..tracks {
            if r.take(4)? != b"MTrk" {
                return Err(Error::InvalidEvents);
            }
            let len = r.u32()? as usize;
            let mut tr = Reader {
                data: r.take(len)?,
                at: 0,
            };
            let (mut tick, mut running) = (0_u64, 0_u8);
            let mut ended = false;
            while tr.at < tr.data.len() {
                tick = tick
                    .checked_add(tr.vlq()? as u64)
                    .ok_or(Error::InvalidEvents)?;
                let first = tr.byte()?;
                let status = if first & 128 != 0 {
                    first
                } else {
                    if running == 0 {
                        return Err(Error::InvalidEvents);
                    }
                    tr.at -= 1;
                    running
                };
                match status {
                    0xff => {
                        running = 0;
                        let kind = tr.byte()?;
                        let len = tr.vlq()? as usize;
                        let v = tr.take(len)?;
                        if kind == 0x2f {
                            if len != 0 {
                                return Err(Error::InvalidEvents);
                            }
                            ended = true;
                            break;
                        }
                        if kind == 0x51 {
                            if len != 3 {
                                return Err(Error::InvalidEvents);
                            }
                            let micros = u32::from_be_bytes([0, v[0], v[1], v[2]]);
                            if micros == 0 {
                                return Err(Error::InvalidEvents);
                            }
                            tempos.push(TempoChange {
                                tick,
                                micros_per_quarter: micros,
                            });
                        }
                    }
                    0xf0 | 0xf7 => {
                        running = 0;
                        let n = tr.vlq()? as usize;
                        tr.take(n)?;
                    }
                    0x80..=0xef => {
                        running = status;
                        let channel = status & 15;
                        let a = tr.byte()?;
                        let b = if status >> 4 == 0xc || status >> 4 == 0xd {
                            0
                        } else {
                            tr.byte()?
                        };
                        if a > 127 || b > 127 {
                            return Err(Error::InvalidEvents);
                        }
                        let message = match status >> 4 {
                            8 => Some(MidiMessage::NoteOff {
                                channel,
                                note: a,
                                velocity: b,
                            }),
                            9 if b == 0 => Some(MidiMessage::NoteOff {
                                channel,
                                note: a,
                                velocity: 0,
                            }),
                            9 => Some(MidiMessage::NoteOn {
                                channel,
                                note: a,
                                velocity: b,
                            }),
                            10 => Some(MidiMessage::PolyPressure {
                                channel,
                                note: a,
                                value: b,
                            }),
                            11 => Some(MidiMessage::ControlChange {
                                channel,
                                controller: a,
                                value: b,
                            }),
                            13 => Some(MidiMessage::ChannelPressure { channel, value: a }),
                            14 => Some(MidiMessage::PitchBend {
                                channel,
                                value: a as u16 | ((b as u16) << 7),
                            }),
                            _ => None,
                        };
                        if let Some(message) = message {
                            events.push(ClipEvent { tick, message });
                        }
                    }
                    _ => return Err(Error::InvalidEvents),
                }
                if events.len() + tempos.len() > 131072 {
                    return Err(Error::InvalidEvents);
                }
            }
            if !ended {
                return Err(Error::InvalidEvents);
            }
        }
        if r.at != r.data.len() {
            return Err(Error::InvalidEvents);
        }
        events.sort_by_key(|e| e.tick);
        tempos.sort_by_key(|e| e.tick);
        Ok(Self {
            ppqn,
            tempos,
            events,
        })
    }
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        if self.ppqn == 0
            || self.ppqn & 0x8000 != 0
            || self.events.len() + self.tempos.len() > 131072
        {
            return Err(Error::InvalidEvents);
        }
        let mut tempos = self.tempos.clone();
        tempos.sort_by_key(|e| e.tick);
        let mut a = vec![];
        let mut at = 0;
        for t in tempos {
            if t.micros_per_quarter == 0 || t.micros_per_quarter > 0xff_ffff {
                return Err(Error::InvalidEvents);
            }
            vlq(t.tick - at, &mut a)?;
            a.extend_from_slice(&[0xff, 0x51, 3]);
            a.extend_from_slice(&t.micros_per_quarter.to_be_bytes()[1..]);
            at = t.tick;
        }
        a.extend_from_slice(&[0, 0xff, 0x2f, 0]);
        let mut events = self.events.clone();
        events.sort_by_key(|e| e.tick);
        let mut b = vec![];
        at = 0;
        for e in events {
            let bytes = encode(e.message)?;
            vlq(e.tick - at, &mut b)?;
            b.extend_from_slice(&bytes[..if bytes[0] >> 4 == 0xd { 2 } else { 3 }]);
            at = e.tick;
        }
        b.extend_from_slice(&[0, 0xff, 0x2f, 0]);
        let mut out = vec![];
        out.extend_from_slice(b"MThd\0\0\0\x06\0\x01\0\x02");
        out.extend_from_slice(&self.ppqn.to_be_bytes());
        for t in [a, b] {
            out.extend_from_slice(b"MTrk");
            out.extend_from_slice(&(t.len() as u32).to_be_bytes());
            out.extend_from_slice(&t);
        }
        Ok(out)
    }
    pub fn frame_events(&self, rate: f64) -> Result<Vec<(u64, MidiMessage)>, Error> {
        if self.ppqn == 0
            || self.ppqn & 0x8000 != 0
            || !rate.is_finite()
            || !(8000.0..=192000.0).contains(&rate)
            || self.events.len() + self.tempos.len() > 131072
        {
            return Err(Error::InvalidEvents);
        }
        let mut events = self.events.clone();
        events.sort_by_key(|e| e.tick);
        let mut tempos = self.tempos.clone();
        tempos.sort_by_key(|e| e.tick);
        let (mut ti, mut tick, mut micros, mut elapsed) = (0, 0_u64, 500000_u32, 0.0);
        let mut out = vec![];
        for e in events {
            while ti < tempos.len() && tempos[ti].tick <= e.tick {
                let t = tempos[ti];
                if t.micros_per_quarter == 0 || t.micros_per_quarter > 0xff_ffff {
                    return Err(Error::InvalidEvents);
                }
                elapsed += (t.tick - tick) as f64 * micros as f64 / self.ppqn as f64;
                tick = t.tick;
                micros = t.micros_per_quarter;
                ti += 1;
            }
            let frame =
                (elapsed + (e.tick - tick) as f64 * micros as f64 / self.ppqn as f64) * rate / 1e6;
            if frame > u64::MAX as f64 {
                return Err(Error::InvalidEvents);
            }
            out.push((frame.round() as u64, e.message));
        }
        Ok(out)
    }
}
fn encode(m: MidiMessage) -> Result<[u8; 3], Error> {
    soniclaw_core::validate_events(
        &[soniclaw_core::Event {
            frame: 0,
            message: m,
        }],
        1,
    )?;
    Ok(match m {
        MidiMessage::NoteOn {
            channel,
            note,
            velocity,
        } => [0x90 | channel, note, velocity],
        MidiMessage::NoteOff {
            channel,
            note,
            velocity,
        } => [0x80 | channel, note, velocity],
        MidiMessage::NoteOnId {
            channel,
            note,
            velocity,
            ..
        } => [0x90 | channel, note, (velocity * 127.0).round() as u8],
        MidiMessage::NoteOffId {
            target:
                NoteTarget {
                    channel: 0..=15,
                    key: 0..=127,
                    ..
                },
        }
        | MidiMessage::NoteChoke {
            target:
                NoteTarget {
                    channel: 0..=15,
                    key: 0..=127,
                    ..
                },
        } => {
            let target = match m {
                MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => target,
                _ => unreachable!(),
            };
            [0x80 | target.channel as u8, target.key as u8, 0]
        }
        MidiMessage::ControlChange {
            channel,
            controller,
            value,
        } => [0xb0 | channel, controller, value],
        MidiMessage::PitchBend { channel, value } => {
            [0xe0 | channel, (value & 127) as u8, (value >> 7) as u8]
        }
        MidiMessage::ChannelPressure { channel, value } => [0xd0 | channel, value, 0],
        MidiMessage::PolyPressure {
            channel,
            note,
            value,
        } => [0xa0 | channel, note, value],
        _ => return Err(Error::InvalidEvents),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ppqn_tempo_roundtrip_and_truncation_rejection() {
        let c = MidiClip {
            ppqn: 480,
            tempos: vec![
                TempoChange {
                    tick: 0,
                    micros_per_quarter: 500000,
                },
                TempoChange {
                    tick: 480,
                    micros_per_quarter: 1000000,
                },
            ],
            events: vec![
                ClipEvent {
                    tick: 0,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                },
                ClipEvent {
                    tick: 960,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                },
            ],
        };
        let bytes = c.write().unwrap();
        let d = MidiClip::read(&bytes).unwrap();
        assert_eq!(d.events, c.events);
        assert_eq!(d.tempos, c.tempos);
        assert_eq!(d.frame_events(48000.0).unwrap()[1].0, 72000);
        for n in 0..bytes.len() {
            assert!(MidiClip::read(&bytes[..n]).is_err());
        }
    }
}
