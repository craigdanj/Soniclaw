//! MIDI 1 channel messages. Running status and interspersed real-time bytes work;
//! SysEx/system-common/program-change/aftertouch are consumed but not emitted.
use soniclaw_core::MidiMessage;

#[derive(Default)]
pub struct Parser {
    status: u8,
    data: [u8; 2],
    count: usize,
    sysex: bool,
}
impl Parser {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn feed(&mut self, byte: u8) -> Option<MidiMessage> {
        if byte >= 0xf8 {
            return None;
        } // Clock etc. must not disrupt running status.
        if byte & 0x80 != 0 {
            self.count = 0;
            if byte == 0xf0 {
                self.sysex = true;
                self.status = 0;
                return None;
            }
            if byte == 0xf7 {
                self.sysex = false;
                self.status = 0;
                return None;
            }
            self.sysex = false;
            self.status = if byte < 0xf0 { byte } else { 0 };
            return None;
        }
        if self.sysex || self.status == 0 {
            return None;
        }
        self.data[self.count] = byte;
        self.count += 1;
        let kind = self.status >> 4;
        let required = if kind == 0xc || kind == 0xd { 1 } else { 2 };
        if self.count < required {
            return None;
        }
        self.count = 0;
        let channel = self.status & 0xf;
        let [a, b] = self.data;
        match kind {
            0x8 => Some(MidiMessage::NoteOff {
                channel,
                note: a,
                velocity: b,
            }),
            0x9 if b == 0 => Some(MidiMessage::NoteOff {
                channel,
                note: a,
                velocity: 0,
            }),
            0x9 => Some(MidiMessage::NoteOn {
                channel,
                note: a,
                velocity: b,
            }),
            0xb => Some(MidiMessage::ControlChange {
                channel,
                controller: a,
                value: b,
            }),
            0xe => Some(MidiMessage::PitchBend {
                channel,
                value: a as u16 | ((b as u16) << 7),
            }),
            _ => None,
        }
    }
}

#[cfg(feature = "native")]
pub mod native {
    use super::*;
    pub type MidiError = Box<dyn std::error::Error>;
    pub type Connection = midir::MidiInputConnection<Parser>;
    pub fn input_ports() -> Result<Vec<String>, MidiError> {
        let input = midir::MidiInput::new("Soniclaw ports")?;
        input
            .ports()
            .iter()
            .map(|p| input.port_name(p).map_err(Into::into))
            .collect()
    }
    /// Keep this connection alive. The callback runs on the MIDI thread; bridge
    /// to audio with a bounded queue, never lock the processor from this callback.
    pub fn connect_input(
        index: usize,
        mut on_message: impl FnMut(MidiMessage) + Send + 'static,
    ) -> Result<Connection, MidiError> {
        let mut input = midir::MidiInput::new("Soniclaw MIDI")?;
        input.ignore(midir::Ignore::None);
        let port = input
            .ports()
            .get(index)
            .cloned()
            .ok_or("MIDI port index out of range")?;
        Ok(input.connect(
            &port,
            "Soniclaw input",
            move |_, bytes, parser| {
                for &byte in bytes {
                    if let Some(message) = parser.feed(byte) {
                        on_message(message);
                    }
                }
            },
            Parser::default(),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(bytes: &[u8]) -> Vec<MidiMessage> {
        let mut p = Parser::default();
        bytes.iter().filter_map(|b| p.feed(*b)).collect()
    }
    #[test]
    fn running_status_and_clock() {
        let events = parse(&[0x92, 60, 100, 62, 0xf8, 0]);
        assert_eq!(
            events,
            vec![
                MidiMessage::NoteOn {
                    channel: 2,
                    note: 60,
                    velocity: 100
                },
                MidiMessage::NoteOff {
                    channel: 2,
                    note: 62,
                    velocity: 0
                }
            ]
        );
    }
    #[test]
    fn sysex_and_common_cancel_running_status() {
        assert_eq!(
            parse(&[0x90, 60, 0xf0, 1, 2, 0xf8, 0xf7, 60, 100, 0xf2, 1, 2]),
            vec![]
        );
    }
    #[test]
    fn pitch_bend_and_one_byte_messages() {
        assert_eq!(
            parse(&[0xc0, 3, 4, 0xe0, 0, 64]),
            vec![MidiMessage::PitchBend {
                channel: 0,
                value: 8192
            }]
        );
    }
}
