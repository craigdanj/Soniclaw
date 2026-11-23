use super::*;
use soniclaw_dsp::multimic_sampler::{MultiMicSampler, Take, Zone};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let zones = vec![Zone {
        note: 38,
        velocities: [1, 127],
        choke_group: None,
        takes: (0..3)
            .map(|take| {
                let microphones = [
                    (0, 0.58, 0.00048),
                    (37, 0.33, 0.00031),
                    (130, 0.21, 0.00015),
                ]
                .into_iter()
                .map(|(delay, level, loss)| {
                    let mut seed = 239u32 + take;
                    (0..16_000)
                        .map(|i| {
                            if i < delay {
                                return 0.;
                            }
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            let t = (i - delay) as f32;
                            let noise = 2. * (seed as f32 / u32::MAX as f32) - 1.;
                            let value = 0.65 * noise
                                + 0.35
                                    * (std::f32::consts::TAU * (183. + take as f32 * 16.) * t
                                        / 48_000.)
                                        .cos();
                            value * level * (-loss * t).exp()
                        })
                        .collect::<Arc<[f32]>>()
                })
                .collect();
                Take { microphones }
            })
            .collect(),
    }];
    let mut sampler = MultiMicSampler::new(48_000, 3, 128, 16, zones)?;
    let mut stems = vec![vec![0.; 64_000]; 3];
    let [close, overhead, room] = stems.as_mut_slice() else {
        unreachable!()
    };
    for (i, ((a, b), c)) in close
        .chunks_mut(128)
        .zip(overhead.chunks_mut(128))
        .zip(room.chunks_mut(128))
        .enumerate()
    {
        let start = i * 128;
        let mut event = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }];
        let count = if let Some((offset, velocity)) =
            [(123, 127), (12_011, 88), (24_583, 113), (42_421, 127)]
                .into_iter()
                .find(|(t, _)| (start..start + a.len()).contains(t))
        {
            event[0] = Event {
                frame: offset - start,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 38,
                    velocity,
                },
            };
            1
        } else {
            0
        };
        let memory = allocation_counter::measure(|| {
            sampler.process(&mut [a, b, c], &event[..count]).unwrap()
        });
        assert_eq!(memory.count_total, 0);
    }
    let onsets: Vec<_> = stems
        .iter()
        .map(|s| s.iter().position(|x| x.abs() > 1e-7).unwrap())
        .collect();
    assert_eq!(onsets, [123, 160, 253]);
    for (name, stem) in ["close.wav", "overhead.wav", "room.wav"].iter().zip(stems) {
        let mut w = hound::WavWriter::create(
            out.join(name),
            hound::WavSpec {
                sample_rate: 48_000,
                channels: 1,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in stem {
            w.write_sample(x)?;
        }
        w.finalize()?;
    }
    Ok(
        serde_json::json!({"onsets":onsets,"microphones":3,"callback_allocations":0,"round_robin_takes":3}),
    )
}
