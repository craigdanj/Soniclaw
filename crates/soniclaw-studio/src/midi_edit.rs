//! Control-side controller edits and state projection for loop takes.
use crate::{
    midi_recording::{Captured, Message, Take},
    project::{Project, ProjectError},
};
pub use soniclaw_clips::{Control, ControlEvent};
use std::collections::BTreeMap;
pub fn upsert(p: &mut Project, clip: &str, point: ControlEvent) -> Result<(), ProjectError> {
    let mut draft = p.edit_view()?;
    draft.document.apply(soniclaw_clips::Edit::UpsertControl {
        clip: clip.into(),
        point,
    })?;
    p.replace_edit_view(draft)
}
pub fn remove(p: &mut Project, clip: &str, id: u32) -> Result<(), ProjectError> {
    let mut draft = p.edit_view()?;
    draft.document.apply(soniclaw_clips::Edit::RemoveControl {
        clip: clip.into(),
        id,
    })?;
    p.replace_edit_view(draft)
}
pub(crate) fn prefix(take: &Take, start: u64, source: u64) -> Vec<Captured> {
    let corrected =
        |sample: u64| sample.saturating_add_signed(-(take.settings.input_offset_frames as i64));
    let mut held = BTreeMap::<(u8, u8), Vec<(Message, bool)>>::new();
    let mut pedal = [false; 16];
    let mut states = BTreeMap::<(u8, u16), Message>::new();
    for e in take.events.iter().filter(|e| corrected(e.sample) < start) {
        match e.message {
            Message::On {
                channel,
                key,
                velocity,
            } if velocity > 0 => {
                let voices = held.entry((channel, key)).or_default();
                if pedal[channel as usize] {
                    for (_, down) in voices.iter_mut() {
                        *down = false;
                    }
                } else {
                    voices.clear();
                }
                voices.push((e.message, true));
            }
            Message::On { channel, key, .. } | Message::Off { channel, key, .. } => {
                if let Some(voices) = held.get_mut(&(channel, key)) {
                    for (_, down) in voices.iter_mut() {
                        *down = false;
                    }
                    if !pedal[channel as usize] {
                        voices.clear();
                    }
                }
            }
            Message::Control {
                channel,
                controller: 64,
                value,
            } => {
                pedal[channel as usize] = value >= 64;
                if value < 64 {
                    for ((ch, _), voices) in &mut held {
                        if *ch == channel {
                            voices.retain(|(_, down)| *down);
                        }
                    }
                }
                states.insert((channel, 64), e.message);
            }
            Message::Control {
                channel,
                controller: 120,
                ..
            } => held.retain(|(ch, _), _| *ch != channel),
            Message::Control {
                channel,
                controller: 123,
                ..
            } => {
                for ((ch, _), voices) in &mut held {
                    if *ch == channel {
                        if pedal[channel as usize] {
                            for (_, down) in voices {
                                *down = false;
                            }
                        } else {
                            voices.clear();
                        }
                    }
                }
            }
            Message::Control {
                channel,
                controller: 121,
                ..
            } => {
                pedal[channel as usize] = false;
                states.retain(|(ch, _), _| *ch != channel);
                for ((ch, _), voices) in &mut held {
                    if *ch == channel {
                        voices.retain(|(_, down)| *down);
                    }
                }
            }
            Message::Control {
                channel,
                controller,
                ..
            } if controller < 120 => {
                states.insert((channel, controller as u16), e.message);
            }
            Message::Bend { channel, .. } => {
                states.insert((channel, 128), e.message);
            }
            Message::Pressure { channel, .. } => {
                states.insert((channel, 129), e.message);
            }
            Message::PolyPressure { channel, key, .. } => {
                states.insert((channel, 130 + key as u16), e.message);
            }
            Message::Panic => {
                held.clear();
                pedal.fill(false);
                for channel in 0..16 {
                    states.insert(
                        (channel, 64),
                        Message::Control {
                            channel,
                            controller: 64,
                            value: 0,
                        },
                    );
                }
            }
            _ => {}
        }
    }
    let mut events = states
        .values()
        .map(|&message| Captured {
            sample: source,
            message,
            source_sample: None,
            chased: true,
        })
        .collect::<Vec<_>>();
    for ((channel, key), voices) in held {
        for (message, down) in voices {
            events.push(Captured {
                sample: source,
                message,
                source_sample: None,
                chased: true,
            });
            if !down {
                events.push(Captured {
                    sample: source,
                    message: Message::Off {
                        channel,
                        key,
                        velocity: 0,
                    },
                    source_sample: None,
                    chased: true,
                });
            }
        }
    }
    events
}
