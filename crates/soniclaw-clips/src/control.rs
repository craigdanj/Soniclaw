//! Portable MIDI controller points and bounded deterministic seek chase.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Control {
    Cc { controller: u8, value: u8 },
    Bend { value: u16 },
    Pressure { value: u8 },
    PolyPressure { key: u8, value: u8 },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEvent {
    pub id: u32,
    pub beat: f64,
    pub channel: u8,
    pub control: Control,
}
impl ControlEvent {
    pub fn validate(&self, length: f64) -> Result<(), Error> {
        if !self.beat.is_finite() || self.beat < 0. || self.beat >= length || self.channel > 15 {
            return Err(Error::InvalidEvents);
        }
        match self.control {
            Control::Cc { controller, value } if controller > 127 || value > 127 => {
                Err(Error::InvalidEvents)
            }
            Control::Bend { value } if value > 16383 => Err(Error::InvalidEvents),
            Control::Pressure { value } if value > 127 => Err(Error::InvalidEvents),
            Control::PolyPressure { key, value } if key > 127 || value > 127 => {
                Err(Error::InvalidEvents)
            }
            _ => Ok(()),
        }
    }
    pub fn message(&self) -> MidiMessage {
        let channel = self.channel;
        match self.control {
            Control::Cc { controller, value } => MidiMessage::ControlChange {
                channel,
                controller,
                value,
            },
            Control::Bend { value } => MidiMessage::PitchBend { channel, value },
            Control::Pressure { value } => MidiMessage::ChannelPressure { channel, value },
            Control::PolyPressure { key, value } => MidiMessage::PolyPressure {
                channel,
                note: key,
                value,
            },
        }
    }
}
fn key(m: MidiMessage) -> Option<(u8, u16)> {
    match m {
        MidiMessage::ControlChange {
            channel,
            controller,
            ..
        } if controller < 120 => Some((channel, controller as u16)),
        MidiMessage::PitchBend { channel, .. } => Some((channel, 128)),
        MidiMessage::ChannelPressure { channel, .. } => Some((channel, 129)),
        MidiMessage::PolyPressure { channel, note, .. } => Some((channel, 130 + note as u16)),
        _ => None,
    }
}
fn neutral(m: MidiMessage) -> MidiMessage {
    match m {
        MidiMessage::ControlChange {
            channel,
            controller,
            ..
        } => MidiMessage::ControlChange {
            channel,
            controller,
            value: match controller {
                7 => 100,
                10 => 64,
                11 => 127,
                70..=79 => 64,
                _ => 0,
            },
        },
        MidiMessage::PitchBend { channel, .. } => MidiMessage::PitchBend {
            channel,
            value: 8192,
        },
        MidiMessage::ChannelPressure { channel, .. } => {
            MidiMessage::ChannelPressure { channel, value: 0 }
        }
        MidiMessage::PolyPressure { channel, note, .. } => MidiMessage::PolyPressure {
            channel,
            note,
            value: 0,
        },
        _ => m,
    }
}
struct Lane {
    key: (u8, u16),
    default: MidiMessage,
    points: Vec<Scheduled>,
    next_low: Vec<u64>,
}
pub(crate) struct Plan {
    lanes: Vec<Lane>,
    off: [Vec<u64>; 16],
    kill: [Vec<u64>; 16],
    pub has_sustain: bool,
}
impl Plan {
    pub fn compile(
        clips: &[MidiClip],
        map: &TempoMap,
        rate: f64,
    ) -> Result<(Self, Vec<Scheduled>), Error> {
        let mut events = vec![];
        let mut work = 0usize;
        for c in clips.iter().filter(|c| c.enabled && !c.controls.is_empty()) {
            let mut keys = BTreeMap::new();
            for point in &c.controls {
                let m = point.message();
                if let Some(key) = key(m) {
                    keys.insert(key, neutral(m));
                }
            }
            let mut points = c.controls.iter().collect::<Vec<_>>();
            points.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
            let repeats = if c.looping {
                ((c.length_beats + c.source_offset) / c.source_length).ceil() as usize
            } else {
                1
            };
            work = work
                .checked_add(
                    repeats
                        .checked_mul(c.controls.len() + keys.len() * 2)
                        .ok_or(Error::InvalidEvents)?,
                )
                .ok_or(Error::InvalidEvents)?;
            if repeats > 1_000_000 || work > 1_000_000 {
                return Err(Error::InvalidEvents);
            }
            for repeat in 0..repeats {
                let base = repeat as f64 * c.source_length - c.source_offset;
                let a = base.max(0.);
                let b = (base + c.source_length).min(c.length_beats);
                if a >= b {
                    continue;
                }
                let phase = a - base;
                let start = map.sample_at(c.start_beat + a, rate)?;
                let end = map.sample_at(c.start_beat + b, rate)?;
                for (&key, &default) in &keys {
                    let initial = points
                        .iter()
                        .rev()
                        .find(|p| p.beat <= phase && self::key(p.message()) == Some(key))
                        .map_or(default, |p| p.message());
                    events.push(Scheduled {
                        sample: start,
                        message: initial,
                        priority: if matches!(initial, MidiMessage::PolyPressure { .. }) {
                            5
                        } else {
                            3
                        },
                    });
                    events.push(Scheduled {
                        sample: end,
                        message: default,
                        priority: 2,
                    });
                }
                for point in &points {
                    let local = base + point.beat;
                    if local >= a && local < b {
                        let message = point.message();
                        events.push(Scheduled {
                            sample: map.sample_at(c.start_beat + local, rate)?,
                            message,
                            priority: if matches!(message, MidiMessage::PolyPressure { .. }) {
                                5
                            } else {
                                3
                            },
                        });
                    }
                }
            }
        }
        events.sort_by_key(|e| (e.sample, e.priority));
        let mut lanes = BTreeMap::<(u8, u16), Vec<Scheduled>>::new();
        let mut off: [Vec<u64>; 16] = std::array::from_fn(|_| vec![]);
        let mut kill: [Vec<u64>; 16] = std::array::from_fn(|_| vec![]);
        let mut reset_commands: [Vec<u64>; 16] = std::array::from_fn(|_| vec![]);
        for e in &events {
            if let Some(key) = key(e.message) {
                lanes.entry(key).or_default().push(*e);
            }
            match e.message {
                MidiMessage::ControlChange {
                    channel,
                    controller: 120,
                    ..
                } => kill[channel as usize].push(e.sample),
                MidiMessage::ControlChange {
                    channel,
                    controller: 123,
                    ..
                } => off[channel as usize].push(e.sample),
                MidiMessage::ControlChange {
                    channel,
                    controller: 121,
                    ..
                } => reset_commands[channel as usize].push(e.sample),
                _ => {}
            }
        }
        if lanes.len() > 128 {
            return Err(Error::InvalidEvents);
        }
        let has_sustain = lanes.keys().any(|(_, kind)| *kind == 64);
        let mut prepared = vec![];
        for (key, mut points) in lanes {
            let default = neutral(points[0].message);
            for &sample in &reset_commands[key.0 as usize] {
                points.push(Scheduled {
                    sample,
                    message: default,
                    priority: 3,
                });
            }
            points.sort_by_key(|e| (e.sample, e.priority));
            let mut next_low = vec![u64::MAX; points.len() + 1];
            for i in (0..points.len()).rev() {
                next_low[i] = if matches!(
                    points[i].message,
                    MidiMessage::ControlChange {
                        controller: 64,
                        value: 0..=63,
                        ..
                    }
                ) {
                    points[i].sample
                } else {
                    next_low[i + 1]
                };
            }
            prepared.push(Lane {
                key,
                default,
                points,
                next_low,
            });
        }
        Ok((
            Self {
                lanes: prepared,
                off,
                kill,
                has_sustain,
            },
            events,
        ))
    }
    pub fn count(&self) -> usize {
        self.lanes.len()
    }
    pub fn chase(&self, sample: u64, out: &mut Vec<Event>, after_notes: bool) {
        for lane in &self.lanes {
            if (lane.key.1 >= 130) != after_notes {
                continue;
            }
            let i = lane.points.partition_point(|p| p.sample < sample);
            let message = if i == 0 {
                lane.default
            } else {
                lane.points[i - 1].message
            };
            out.push(Event { frame: 0, message });
        }
    }
    pub fn reset(&self, out: &mut Vec<Event>) {
        for lane in &self.lanes {
            out.push(Event {
                frame: 0,
                message: lane.default,
            });
        }
    }
    pub fn extend(&self, span: &mut Span) {
        let channel = match span.on {
            MidiMessage::NoteOnId { channel, .. } => channel as usize,
            _ => return,
        };
        let next = |values: &[u64]| {
            values
                .get(values.partition_point(|s| *s <= span.start))
                .copied()
                .unwrap_or(u64::MAX)
        };
        let kill = next(&self.kill[channel]);
        span.key_end = span.key_end.min(next(&self.off[channel]));
        let mut end = span.key_end;
        if let Some(lane) = self.lanes.iter().find(|l| l.key == (channel as u8, 64)) {
            let i = lane.points.partition_point(|p| p.sample < span.key_end);
            if i > 0
                && matches!(
                    lane.points[i - 1].message,
                    MidiMessage::ControlChange {
                        value: 64..=127,
                        ..
                    }
                )
            {
                end = lane.next_low[i];
            }
        }
        span.end = end.min(kill);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        Document {
            schema: 1,
            sample_rate: 48000,
            channels: 2,
            timeline: Default::default(),
            audio: vec![],
            midi: vec![MidiClip {
                controls: vec![
                    ControlEvent {
                        id: 0,
                        beat: 0.001,
                        channel: 0,
                        control: Control::Cc {
                            controller: 64,
                            value: 127,
                        },
                    },
                    ControlEvent {
                        id: 1,
                        beat: 0.006,
                        channel: 0,
                        control: Control::Bend { value: 10000 },
                    },
                    ControlEvent {
                        id: 2,
                        beat: 0.007,
                        channel: 0,
                        control: Control::PolyPressure {
                            key: 60,
                            value: 100,
                        },
                    },
                    ControlEvent {
                        id: 3,
                        beat: 0.03,
                        channel: 0,
                        control: Control::Cc {
                            controller: 64,
                            value: 0,
                        },
                    },
                ],
                id: "expression".into(),
                track: 0,
                start_beat: 0.,
                length_beats: 0.04,
                source_offset: 0.,
                source_length: 0.04,
                looping: false,
                notes: vec![Note {
                    id: 0,
                    beat: 0.,
                    length: 0.01,
                    channel: 0,
                    key: 60,
                    velocity: 1.,
                }],
                enabled: true,
            }],
        }
    }
    fn render(block: usize) -> Vec<(u64, MidiMessage)> {
        let d = document();
        let mut p = d.compile(&Assets::new(), 128, None).unwrap();
        let mut pcm = vec![0.; 256];
        let mut result = vec![];
        for start in (0..1024).step_by(block) {
            let n = block.min(1024 - start);
            p.render(
                &mut AudioBuffer::new(&mut pcm[..n * 2], 2).unwrap(),
                Transport {
                    sample_position: start as u64,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
            for e in p.last_events() {
                result.push((start as u64 + e.frame as u64, e.message));
            }
        }
        result
    }
    #[test]
    fn controllers_offsets_resets_and_partition_are_exact() {
        let a = render(64);
        assert_eq!(a, render(127));
        assert!(a
            .iter()
            .any(|(s, m)| *s == 144 && matches!(m, MidiMessage::PitchBend { value: 10000, .. })));
        assert!(a
            .iter()
            .any(|(s, m)| *s == 960 && matches!(m, MidiMessage::PitchBend { value: 8192, .. })));
    }
    #[test]
    fn seek_chases_sustained_key_after_key_up_without_stuck_note() {
        let d = document();
        let mut p = d.compile(&Assets::new(), 128, None).unwrap();
        let mut pcm = vec![0.; 128];
        let measured = allocation_counter::measure(|| {
            p.render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 480,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap()
        });
        assert_eq!(measured.count_total, 0);
        let e = p.last_events();
        let on = e
            .iter()
            .position(|e| matches!(e.message, MidiMessage::NoteOnId { note: 60, .. }))
            .unwrap();
        let off = e
            .iter()
            .position(|e| matches!(e.message, MidiMessage::NoteOffId { .. }))
            .unwrap();
        let pressure = e
            .iter()
            .position(|e| matches!(e.message, MidiMessage::PolyPressure { value: 100, .. }))
            .unwrap();
        assert!(on < off && off < pressure);
        assert!(e[..on].iter().any(|e| matches!(
            e.message,
            MidiMessage::ControlChange {
                controller: 64,
                value: 127,
                ..
            }
        )));
        p.render(
            &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
            Transport {
                sample_position: 800,
                playing: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!p
            .last_events()
            .iter()
            .any(|e| matches!(e.message, MidiMessage::NoteOnId { .. })));
    }
    #[test]
    fn edits_are_atomic_and_legacy_empty_controls_are_readable() {
        let mut d = document();
        let before = d.to_json().unwrap();
        assert!(d
            .apply(Edit::UpsertControl {
                clip: "expression".into(),
                point: ControlEvent {
                    id: 9,
                    beat: 1.,
                    channel: 0,
                    control: Control::Bend { value: 1 }
                }
            })
            .is_err());
        assert_eq!(before, d.to_json().unwrap());
        d.midi[0].controls.clear();
        let text = d.to_json().unwrap();
        assert!(!text.contains("controls"));
        assert!(Document::from_json(&text).unwrap().midi[0]
            .controls
            .is_empty());
    }
}
