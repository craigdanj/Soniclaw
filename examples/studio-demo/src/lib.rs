use soniclaw::{automation, clips::*, mixer, studio::*};
use std::{collections::BTreeMap, sync::Arc};
pub fn fixture() -> (soniclaw::studio::Document, Assets) {
    let rate = 48000;
    let assets = Assets::from([(
        "bass".into(),
        AudioAsset {
            sample_rate: rate,
            channels: 1,
            samples: Arc::from(
                (0..rate)
                    .map(|i| (i as f32 * 110. * std::f32::consts::TAU / rate as f32).sin() * 0.15)
                    .collect::<Vec<_>>(),
            ),
        },
    )]);
    let mut d = soniclaw::studio::Document {
        cues: Vec::new(),
        mix_routing: Default::default(),
        schema: 1,
        sample_rate: rate,
        max_block_frames: 256,
        parameter_values: BTreeMap::new(),
        timeline: Default::default(),
        tracks: vec![
            Track {
                streamed: vec![],
                id: "bass".into(),
                audio: vec![AudioClip {
                    edits: None,
                    id: "bass-loop".into(),
                    asset: "bass".into(),
                    track: 0,
                    start_frame: 0,
                    length_frames: 96000,
                    source_offset: 0,
                    source_length: 48000,
                    looping: true,
                    gain: 1.,
                    fade_in: 200,
                    fade_out: 200,
                    enabled: true,
                }],
                midi: vec![],
                instrument: None,
                inserts: vec![Descriptor {
                    id: "bass.level".into(),
                    kind: "gain".into(),
                    state: serde_json::json!({"value":0.7}),
                }],
                input_gain: 0.,
                midi_input: false,
            },
            Track {
                streamed: vec![],
                id: "lead".into(),
                audio: vec![],
                midi: vec![MidiClip {
                    controls: vec![],
                    id: "melody".into(),
                    track: 1,
                    start_beat: 0.,
                    length_beats: 4.,
                    source_offset: 0.,
                    source_length: 4.,
                    looping: false,
                    notes: [60, 64, 67, 72]
                        .into_iter()
                        .enumerate()
                        .map(|(i, key)| Note {
                            id: i as u32,
                            beat: i as f64,
                            length: 0.8,
                            channel: 0,
                            key,
                            velocity: 0.7,
                        })
                        .collect(),
                    enabled: true,
                }],
                instrument: Some(Descriptor {
                    id: "lead.synth".into(),
                    kind: "sine".into(),
                    state: serde_json::json!({}),
                }),
                inserts: vec![],
                input_gain: 0.,
                midi_input: true,
            },
        ],
        mixer: mixer::Model {
            schema: 1,
            strips: [("bass", -0.2), ("lead", 0.2)]
                .into_iter()
                .map(|(id, pan)| mixer::Strip {
                    id: id.into(),
                    name: id.into(),
                    gain: 0.8,
                    pan,
                    mute: false,
                    solo: false,
                    group: None,
                    sends: vec![0.15],
                })
                .collect(),
            groups: vec![],
            taps: vec![mixer::Tap::PostFader],
        },
        auxiliaries: vec![Auxiliary {
            inserts: vec![Descriptor {
                id: "echo".into(),
                kind: "delay".into(),
                state: serde_json::json!({"seconds":0.15,"feedback":0.2,"mix":1.}),
            }],
            return_gain: 0.4,
        }],
        automation: automation::Document {
            lanes: vec![automation::Lane {
                parameter: "mix.lead.gain".into(),
                mode: automation::Mode::Read,
                enabled: true,
                points: vec![
                    automation::Point {
                        beat: 0.,
                        value: 0.2,
                        curve: automation::Curve::Linear,
                    },
                    automation::Point {
                        beat: 4.,
                        value: 0.1,
                        curve: automation::Curve::Linear,
                    },
                ],
            }],
            loop_range: None,
        },
    };
    d.timeline.loop_range = Some([0., 4.]);
    (d, assets)
}
