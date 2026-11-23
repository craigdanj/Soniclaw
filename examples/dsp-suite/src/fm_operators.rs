use super::*;
use soniclaw_dsp::fm_operators::{FmSynth, Mode, Patch};

struct Phrase(FmSynth);
impl AudioProcessor for Phrase {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.0.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let mut events = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; 4];
        let start = ctx.transport.sample_position as usize;
        let mut count = 0;
        for (at, message) in [
            (
                128,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 57,
                    velocity: 118,
                },
            ),
            (
                6003,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            ),
            (
                12000,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 64,
                    velocity: 100,
                },
            ),
            (27000, MidiMessage::AllNotesOff),
        ] {
            if (start..start + b.frames()).contains(&at) {
                events[count] = Event {
                    frame: at - start,
                    message,
                };
                count += 1;
            }
        }
        self.0.process(
            b,
            &ProcessContext {
                events: &events[..count],
                transport: ctx.transport,
            },
        );
    }
    fn reset(&mut self) {
        self.0.reset();
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut cases = Vec::new();
    for (name, mode, ratio) in [
        ("fm-bass", Mode::Frequency, 1.),
        ("pm-electric-piano", Mode::Phase, 2.),
        ("pm-bell", Mode::Phase, 3.49),
    ] {
        let mut patch = Patch {
            mode,
            ..Default::default()
        };
        patch.operators[0].ratio = ratio;
        patch.operators[0].feedback = 0.06;
        patch.operators[3].envelope.release = if name == "pm-bell" { 0.45 } else { 0.22 };
        let (report, pcm) = render(
            out,
            name,
            Box::new(Phrase(FmSynth::new(patch, 8)?)),
            vec![0.; 60_000],
        )?;
        assert!(pcm[..128].iter().all(|&v| v == 0.));
        assert!(pcm[57_000..].iter().all(|&v| v == 0.));
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&patch)?,
        )?;
        cases.push(report);
    }
    Ok(serde_json::json!({"cases":cases,"operators":4,"voices":8}))
}
