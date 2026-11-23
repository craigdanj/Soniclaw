//! Loop take projection. Raw acquisition timestamps and all passes are retained.
use crate::{
    midi_recording::{Captured, Pass, Take},
    project::{Project, ProjectError},
    *,
};
impl Take {
    pub fn passes(&self) -> Result<Vec<Pass>, ProjectError> {
        self.validate()?;
        if !self.settings.looping || !self.complete() {
            return Err("complete loop MIDI take required".into());
        }
        let range = self.timeline.loop_range.as_ref().ok_or("missing loop")?;
        let map = TempoMap::compile(&self.timeline)?;
        let start = map.sample_at(range[0], self.sample_rate as f64)?;
        let length = map
            .sample_at(range[1], self.sample_rate as f64)?
            .checked_sub(start)
            .filter(|n| *n > 0)
            .ok_or("invalid MIDI loop span")?;
        let mut result = vec![];
        for (i, c) in self.cycles.iter().enumerate() {
            let end = c.sample.saturating_add(length);
            if c.source == start
                && end <= self.end_sample
                && self.cycles.get(i + 1).is_none_or(|next| next.sample == end)
            {
                result.push(Pass {
                    start_sample: c.sample,
                    end_sample: end,
                    source_sample: start,
                });
            }
        }
        Ok(result)
    }
    /// Return a linear editable/exportable pass, chasing held notes from the preceding raw performance.
    pub fn pass(&self, index: usize) -> Result<Self, ProjectError> {
        let passes = self.passes()?;
        let pass = passes.get(index).ok_or("MIDI loop pass out of range")?;
        let range = self.timeline.loop_range.as_ref().ok_or("missing loop")?;
        let map = TempoMap::compile(&self.timeline)?;
        let end = map.sample_at(range[1], self.sample_rate as f64)?;
        let corrected =
            |sample: u64| sample.saturating_add_signed(-(self.settings.input_offset_frames as i64));
        let mut events = crate::midi_edit::prefix(self, pass.start_sample, pass.source_sample);
        for e in self.events.iter().filter(|e| {
            let sample = corrected(e.sample);
            sample >= pass.start_sample && sample < pass.end_sample
        }) {
            events.push(Captured {
                chased: false,
                sample: pass.source_sample + corrected(e.sample) - pass.start_sample,
                message: e.message,
                source_sample: None,
            });
        }
        if events.len() > 65536 {
            return Err("MIDI pass chase exceeds event bound".into());
        }
        let mut result = self.clone();
        result.timeline.loop_range = None;
        result.settings.looping = false;
        result.settings.start_beat = range[0];
        result.settings.input_offset_frames = 0;
        result.settings.max_events = result.settings.max_events.max(events.len());
        result.start_sample = pass.source_sample;
        result.end_sample = end;
        result.events = events;
        result.cycles.clear();
        result.validate()?;
        Ok(result)
    }
    /// Select/reselect one region for this raw take. An unrelated existing clip is never overwritten.
    pub fn apply_pass(&self, p: &mut Project, index: usize) -> Result<(), ProjectError> {
        use sha2::{Digest, Sha256};
        self.settings.validate(p)?;
        if p.project_id != self.project_id || p.document.timeline != self.timeline {
            return Err("MIDI loop take belongs to another project/timeline".into());
        }
        let fingerprint = format!("{:x}", Sha256::digest(self.to_json()?.as_bytes()));
        let id = &self.settings.clip;
        let selected = p.layout.midi_selections.get(id);
        if selected.is_some_and(|s| s.fingerprint != fingerprint || s.track != self.settings.track)
        {
            return Err("MIDI selection belongs to a different raw take".into());
        }
        if selected.is_none()
            && p.document.tracks.iter().any(|t| {
                t.midi.iter().any(|c| &c.id == id)
                    || t.audio.iter().any(|c| &c.id == id)
                    || t.streamed.iter().any(|c| &c.id == id)
            })
        {
            return Err("MIDI take clip identity collision".into());
        }
        let mut next = p.clone();
        let track = next
            .document
            .tracks
            .iter()
            .position(|t| t.id == self.settings.track)
            .ok_or("MIDI destination missing")?;
        let clip = self.pass(index)?.note_clip(track as u16)?;
        if clip.notes.is_empty() {
            return Err("selected MIDI pass contains no notes".into());
        }
        next.document.tracks[track].midi.retain(|c| &c.id != id);
        next.document.tracks[track].midi.push(clip);
        next.layout.midi_selections.insert(
            id.clone(),
            crate::takes::MidiSelection {
                fingerprint,
                pass: index,
                track: self.settings.track.clone(),
            },
        );
        next.validate()?;
        *p = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi_recording::Message;
    fn capture(block: usize) -> (Project, Take) {
        let mut p = workspace::blank("Loop MIDI".into(), 48000, 128).unwrap();
        workspace::apply(
            &mut p,
            workspace::Command::Add {
                id: "keys".into(),
                name: "Keys".into(),
                kind: workspace::Kind::Instrument,
            },
        )
        .unwrap();
        p.document.timeline.loop_range = Some([0., 0.008]);
        let settings = midi_recording::Settings {
            track: "keys".into(),
            clip: "loop-take".into(),
            looping: true,
            ..Default::default()
        };
        let mut a = midi_recording::prepare(&p, &mut Builtins, settings, 0).unwrap();
        let input = [
            (
                10,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            ),
            (
                220,
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 0,
                },
            ),
            (
                250,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 64,
                    velocity: 90,
                },
            ),
            (
                360,
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 64,
                    velocity: 0,
                },
            ),
            (
                400,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 67,
                    velocity: 80,
                },
            ),
            (
                510,
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 67,
                    velocity: 0,
                },
            ),
        ];
        let mut pcm = vec![0.; 256];
        let blocks = (0..640)
            .step_by(block)
            .map(|at| {
                let n = block.min(640 - at);
                let events = input
                    .iter()
                    .filter(|(s, _)| *s >= at && *s < at + n)
                    .map(|(s, m)| Event {
                        frame: s - at,
                        message: *m,
                    })
                    .collect::<Vec<_>>();
                (n, events)
            })
            .collect::<Vec<_>>();
        let measured = allocation_counter::measure(|| {
            for (n, events) in &blocks {
                a.processor.process(
                    &mut AudioBuffer::new(&mut pcm[..n * 2], 2).unwrap(),
                    &ProcessContext {
                        events,
                        transport: Default::default(),
                    },
                );
            }
        });
        assert_eq!(measured.count_total, 0);
        drop(a.processor);
        let take = a.recorder.finish(0).unwrap();
        assert!(take.complete(), "discontinuities {}", take.discontinuities);
        (p, take)
    }
    #[test]
    fn loop_chase_keeps_multiple_pedal_held_voices_and_controller_state() {
        let (_, mut take) = capture(64);
        take.events = [
            (
                10,
                Message::On {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                },
            ),
            (
                20,
                Message::Control {
                    channel: 0,
                    controller: 64,
                    value: 127,
                },
            ),
            (
                40,
                Message::Off {
                    channel: 0,
                    key: 60,
                    velocity: 0,
                },
            ),
            (
                70,
                Message::On {
                    channel: 0,
                    key: 60,
                    velocity: 90,
                },
            ),
            (
                100,
                Message::Off {
                    channel: 0,
                    key: 60,
                    velocity: 0,
                },
            ),
            (
                120,
                Message::Bend {
                    channel: 0,
                    value: 10000,
                },
            ),
            (
                250,
                Message::Control {
                    channel: 0,
                    controller: 64,
                    value: 0,
                },
            ),
        ]
        .into_iter()
        .map(|(sample, message)| Captured {
            sample,
            message,
            source_sample: Some(sample % 192),
            chased: false,
        })
        .collect();
        let pass = take.pass(1).unwrap();
        let clip = pass.note_clip(0).unwrap();
        assert_eq!(clip.notes.len(), 2);
        assert!(clip.notes.iter().all(|n| n.length > 0.));
        assert!(clip.controls.iter().any(|p| matches!(
            p.control,
            soniclaw_clips::Control::Bend { value: 10000 }
        ) && p.beat == 0.));
        let d = soniclaw_clips::Document {
            schema: 1,
            sample_rate: 48000,
            channels: 2,
            timeline: Default::default(),
            audio: vec![],
            midi: vec![clip],
        };
        let mut player = d.compile(&Assets::new(), 128, None).unwrap();
        let mut pcm = vec![0.; 128];
        player
            .render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 20,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            player
                .last_events()
                .iter()
                .filter(|e| matches!(e.message, MidiMessage::NoteOnId { .. }))
                .count(),
            2
        );
    }
    #[test]
    fn passes_are_partition_invariant_and_chase_held_notes() {
        let (mut p, a) = capture(64);
        let (_, b) = capture(127);
        assert_eq!(
            a.cycles.iter().map(|c| c.sample).collect::<Vec<_>>(),
            b.cycles.iter().map(|c| c.sample).collect::<Vec<_>>()
        );
        assert_eq!(a.passes().unwrap().len(), 3);
        assert_eq!(a.pass(1).unwrap().note_clip(1).unwrap().notes.len(), 2);
        assert_eq!(a.pass(2).unwrap().note_clip(1).unwrap().notes[0].key, 67);
        assert!(a.smf(960).is_err());
        assert!(a.pass(1).unwrap().smf(960).is_ok());
        a.apply(&mut p).unwrap();
        assert_eq!(p.layout.midi_selections["loop-take"].pass, 2);
        let mut h = history::History::default();
        h.edit(&mut p, "Choose pass", |p| a.apply_pass(p, 0))
            .unwrap();
        assert_eq!(p.document.tracks[1].midi[0].notes[0].key, 60);
        h.undo(&mut p).unwrap();
        assert_eq!(p.document.tracks[1].midi[0].notes[0].key, 67);
        let roundtrip = Take::from_json(&a.to_json().unwrap()).unwrap();
        assert_eq!(roundtrip.passes().unwrap().len(), 3);
    }
}
