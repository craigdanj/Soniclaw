//! Bounded complete MIDI 1 packets, including program/system messages and SysEx.
use soniclaw_core::Error;
pub const MAX_MESSAGE: usize = 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    bytes: [u8; MAX_MESSAGE],
    len: u16,
}
impl Packet {
    pub fn new(data: &[u8]) -> Result<Self, Error> {
        if data.is_empty() || data.len() > MAX_MESSAGE {
            return Err(Error::InvalidEvents);
        }
        let status = data[0];
        let size = message_size(status);
        let valid = if status == 0xF0 {
            data.len() >= 2
                && data[data.len() - 1] == 0xF7
                && data[1..data.len() - 1].iter().all(|x| *x < 128)
        } else {
            size == Some(data.len()) && data[1..].iter().all(|x| *x < 128)
        };
        if !valid {
            return Err(Error::InvalidEvents);
        }
        let mut bytes = [0; MAX_MESSAGE];
        bytes[..data.len()].copy_from_slice(data);
        Ok(Self {
            bytes,
            len: data.len() as u16,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
    pub fn to_core(&self) -> Option<soniclaw_core::MidiMessage> {
        use soniclaw_core::MidiMessage::*;
        let channel = self.bytes[0] & 15;
        let a = self.bytes[1];
        let b = self.bytes[2];
        match self.bytes[0] >> 4 {
            8 => Some(NoteOff {
                channel,
                note: a,
                velocity: b,
            }),
            9 if b == 0 => Some(NoteOff {
                channel,
                note: a,
                velocity: 0,
            }),
            9 => Some(NoteOn {
                channel,
                note: a,
                velocity: b,
            }),
            10 => Some(PolyPressure {
                channel,
                note: a,
                value: b,
            }),
            11 => Some(ControlChange {
                channel,
                controller: a,
                value: b,
            }),
            13 => Some(ChannelPressure { channel, value: a }),
            14 => Some(PitchBend {
                channel,
                value: a as u16 | ((b as u16) << 7),
            }),
            _ => None,
        }
    }
    pub fn all_notes_off(channel: u8) -> Result<Self, Error> {
        if channel > 15 {
            return Err(Error::InvalidEvents);
        }
        Self::new(&[0xB0 | channel, 123, 0])
    }
}
fn message_size(status: u8) -> Option<usize> {
    match status {
        0x80..=0xBF | 0xE0..=0xEF => Some(3),
        0xC0..=0xDF | 0xF1 | 0xF3 => Some(2),
        0xF2 => Some(3),
        0xF6 | 0xF8 | 0xFA..=0xFC | 0xFE..=0xFF => Some(1),
        _ => None,
    }
}
pub struct Parser {
    data: [u8; MAX_MESSAGE],
    len: usize,
    expected: usize,
    running: u8,
    sysex: bool,
    discard: bool,
    pub dropped: u64,
}
impl Default for Parser {
    fn default() -> Self {
        Self {
            data: [0; MAX_MESSAGE],
            len: 0,
            expected: 0,
            running: 0,
            sysex: false,
            discard: false,
            dropped: 0,
        }
    }
}
impl Parser {
    pub fn feed(&mut self, byte: u8) -> Option<Packet> {
        if byte >= 0xF8 {
            return Packet::new(&[byte]).ok();
        }
        if self.sysex {
            if byte == 0xF7 {
                self.sysex = false;
                let result = if self.discard || self.len >= MAX_MESSAGE {
                    None
                } else {
                    self.data[self.len] = byte;
                    Packet::new(&self.data[..self.len + 1]).ok()
                };
                self.len = 0;
                return result;
            }
            if byte < 128 {
                if self.len < MAX_MESSAGE - 1 && !self.discard {
                    self.data[self.len] = byte;
                    self.len += 1;
                } else if !self.discard {
                    self.discard = true;
                    self.dropped += 1;
                }
                return None;
            }
            self.sysex = false;
            self.len = 0;
            self.dropped += 1;
        }
        if byte & 128 != 0 {
            self.len = 0;
            if byte == 0xF0 {
                self.sysex = true;
                self.discard = false;
                self.running = 0;
                self.data[0] = byte;
                self.len = 1;
                return None;
            }
            let Some(size) = message_size(byte) else {
                self.running = 0;
                self.dropped += 1;
                return None;
            };
            self.running = if byte < 0xF0 { byte } else { 0 };
            if size == 1 {
                return Packet::new(&[byte]).ok();
            }
            self.data[0] = byte;
            self.len = 1;
            self.expected = size;
            return None;
        }
        if self.len == 0 {
            if self.running == 0 {
                self.dropped += 1;
                return None;
            }
            self.data[0] = self.running;
            self.len = 1;
            self.expected = message_size(self.running).expect("channel running status");
        }
        self.data[self.len] = byte;
        self.len += 1;
        if self.len == self.expected {
            let p = Packet::new(&self.data[..self.len]).ok();
            self.len = 0;
            p
        } else {
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn running_program_sysex_and_interleaved_clock() {
        let mut p = Parser::default();
        let packets: Vec<_> = [0xC2, 9, 10, 0xF0, 1, 0xF8, 2, 0xF7, 0x92, 60, 100, 61, 0]
            .into_iter()
            .filter_map(|b| p.feed(b))
            .map(|p| p.bytes().to_vec())
            .collect();
        assert_eq!(
            packets,
            vec![
                vec![0xC2, 9],
                vec![0xC2, 10],
                vec![0xF8],
                vec![0xF0, 1, 2, 0xF7],
                vec![0x92, 60, 100],
                vec![0x92, 61, 0]
            ]
        );
    }
    #[test]
    fn sysex_overflow_recovers_without_allocating() {
        let mut p = Parser::default();
        p.feed(0xF0);
        for _ in 0..2048 {
            p.feed(1);
        }
        assert_eq!(p.feed(0xF7), None);
        assert_eq!(p.dropped, 1);
        assert_eq!(p.feed(0xFA).unwrap().bytes(), &[0xFA]);
        assert!(Packet::new(&[0x90, 200, 1]).is_err());
    }
}
