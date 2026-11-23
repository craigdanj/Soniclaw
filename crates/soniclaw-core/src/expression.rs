//! Fixed-size expressive note state shared by instrument implementations.
use crate::{Error, MidiMessage};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteTarget {
    pub note_id: i32,
    pub channel: i16,
    pub key: i16,
}
impl NoteTarget {
    pub fn matches(self, note_id: i32, channel: u8, key: u8) -> bool {
        (self.note_id < 0 || self.note_id == note_id)
            && (self.channel < 0 || self.channel == channel as i16)
            && (self.key < 0 || self.key == key as i16)
    }
    pub fn valid(self) -> bool {
        self.note_id >= -1 && (-1..=15).contains(&self.channel) && (-1..=127).contains(&self.key)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteExpression {
    Tuning,
    Pressure,
    Timbre,
    Pan,
    Volume,
}
#[derive(Clone, Copy, Debug)]
pub struct VoiceExpression {
    pub tuning: f32,
    pub pressure: f32,
    pub timbre: f32,
    pub pan: f32,
    pub volume: f32,
}
impl Default for VoiceExpression {
    fn default() -> Self {
        Self {
            tuning: 0.0,
            pressure: 0.0,
            timbre: 0.5,
            pan: 0.5,
            volume: 1.0,
        }
    }
}
impl VoiceExpression {
    pub fn set(&mut self, expression: NoteExpression, value: f64) -> bool {
        let range = match expression {
            NoteExpression::Tuning => -120.0..=120.0,
            NoteExpression::Pressure | NoteExpression::Timbre | NoteExpression::Pan => 0.0..=1.0,
            NoteExpression::Volume => 0.0..=4.0,
        };
        if !value.is_finite() || !range.contains(&value) {
            return false;
        }
        match expression {
            NoteExpression::Tuning => self.tuning = value as f32,
            NoteExpression::Pressure => self.pressure = value as f32,
            NoteExpression::Timbre => self.timbre = value as f32,
            NoteExpression::Pan => self.pan = value as f32,
            NoteExpression::Volume => self.volume = value as f32,
        };
        true
    }
}
/// An explicitly configured MIDI-1 MPE zone. Channel numbers are zero based.
/// RPN zone negotiation is an application concern; this does not parse MIDI 2 UMP.
#[derive(Clone, Copy, Debug)]
pub struct MpeZone {
    pub master: u8,
    pub first_member: u8,
    pub last_member: u8,
    pub member_bend: f32,
    pub master_bend: f32,
}
impl MpeZone {
    pub fn validate(self) -> Result<Self, Error> {
        if self.master > 15
            || self.first_member > self.last_member
            || self.last_member > 15
            || (self.first_member..=self.last_member).contains(&self.master)
            || ![self.member_bend, self.master_bend]
                .iter()
                .all(|x| x.is_finite() && (0.0..=96.0).contains(x))
        {
            Err(Error::InvalidParameter)
        } else {
            Ok(self)
        }
    }
}
#[derive(Clone)]
pub struct ExpressionChannels {
    bend: [f32; 16],
    pressure: [f32; 16],
    timbre: [f32; 16],
    pub zone: Option<MpeZone>,
    pub bend_range: f32,
}
impl Default for ExpressionChannels {
    fn default() -> Self {
        Self {
            bend: [0.0; 16],
            pressure: [0.0; 16],
            timbre: [0.5; 16],
            zone: None,
            bend_range: 2.0,
        }
    }
}
impl ExpressionChannels {
    pub fn reset(&mut self) {
        self.bend.fill(0.0);
        self.pressure.fill(0.0);
        self.timbre.fill(0.5);
    }
    pub fn update(&mut self, message: MidiMessage) -> bool {
        match message {
            MidiMessage::PitchBend { channel, value } if channel < 16 && value <= 16383 => {
                self.bend[channel as usize] =
                    (value as f32 - 8192.0) / if value >= 8192 { 8191.0 } else { 8192.0 }
            }
            MidiMessage::ChannelPressure { channel, value } if channel < 16 && value <= 127 => {
                self.pressure[channel as usize] = value as f32 / 127.0
            }
            MidiMessage::ControlChange {
                channel,
                controller: 74,
                value,
            } if channel < 16 && value <= 127 => {
                self.timbre[channel as usize] = value as f32 / 127.0
            }
            _ => return false,
        }
        true
    }
    pub fn values(&self, channel: u8) -> (f32, f32, f32) {
        if channel >= 16 {
            return (0.0, 0.0, 0.5);
        }
        let c = channel as usize;
        let (pitch, pressure, timbre) = if let Some(z) = self.zone {
            if (z.first_member..=z.last_member).contains(&channel) {
                (
                    self.bend[c] * z.member_bend + self.bend[z.master as usize] * z.master_bend,
                    self.pressure[c].max(self.pressure[z.master as usize]),
                    self.timbre[c],
                )
            } else {
                (
                    self.bend[c] * self.bend_range,
                    self.pressure[c],
                    self.timbre[c],
                )
            }
        } else {
            (
                self.bend[c] * self.bend_range,
                self.pressure[c],
                self.timbre[c],
            )
        };
        (pitch, pressure, timbre)
    }
}
