use soniclaw::studio::{Built, Descriptor, Factory};
use soniclaw::{
    core::*,
    device::*,
    studio::{
        self,
        cues::{Cue, Send, Source},
        project::{Project, ProjectError},
        workspace,
    },
};
use std::{path::Path, sync::Arc};
struct TestDelay {
    frames: usize,
    data: Vec<f32>,
    cursor: usize,
}
impl AudioProcessor for TestDelay {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.data = vec![0.; self.frames * s.channels];
        self.cursor = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for x in b.samples_mut() {
            std::mem::swap(x, &mut self.data[self.cursor]);
            self.cursor = (self.cursor + 1) % self.data.len();
        }
    }
    fn reset(&mut self) {
        self.data.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.frames
    }
}
fn factory(d: &Descriptor) -> Result<Built, Error> {
    if d.kind == "test.delay" {
        Ok(Built {
            processor: Box::new(TestDelay {
                frames: d.state["frames"].as_u64().ok_or(Error::InvalidParameter)? as usize,
                data: vec![],
                cursor: 0,
            }),
            parameters: vec![],
        })
    } else {
        studio::Builtins.create(d)
    }
}
fn delay(id: &str, frames: usize) -> Descriptor {
    Descriptor {
        id: id.into(),
        kind: "test.delay".into(),
        state: serde_json::json!({"frames":frames}),
    }
}

fn fixture() -> Result<Project, ProjectError> {
    let mut p = workspace::blank("Independent cues".into(), 48000, 128)?;
    let track = p.document.tracks[0].id.clone();
    p.assets.insert(
        "tone".into(),
        soniclaw::clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: Arc::from(vec![0.2; 48000]),
        },
    );
    p.document.tracks[0].audio.push(soniclaw::clips::AudioClip {
        id: "audio".into(),
        asset: "tone".into(),
        track: 0,
        start_frame: 0,
        length_frames: 48000,
        source_offset: 0,
        source_length: 48000,
        looping: false,
        gain: 1.,
        fade_in: 0,
        fade_out: 0,
        enabled: true,
        edits: None,
    });
    p.document.mixer.strips[0].mute = true;
    for (i, (source, outputs)) in [
        (Source::TrackPre(track.clone()), [10, 11]),
        (Source::TrackPost(track), [0, 1]),
        (Source::Input(vec![2]), [6, 7]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut cue = Cue::new(format!("cue-{i}"), format!("Performer {i}"), outputs);
        cue.sends.push(Send {
            id: "send".into(),
            source,
            gain: 1.,
            pan: 0.,
        });
        p.document.cues.push(cue);
    }
    let mut cue = Cue::new("cue-3".into(), "Mixed inputs".into(), [2, 3]);
    cue.sends.push(Send {
        id: "audio".into(),
        source: Source::TrackPre(p.document.tracks[0].id.clone()),
        gain: 1.,
        pan: 0.,
    });
    cue.sends.push(Send {
        id: "input".into(),
        source: Source::Input(vec![0, 1]),
        gain: 0.5,
        pan: 0.,
    });
    p.document.cues.push(cue);
    for cue in &mut p.document.cues {
        cue.metronome = false;
    }
    p.validate()?;
    Ok(p)
}
fn timing_checks() -> Result<(), ProjectError> {
    let mut p = fixture()?;
    let mut signal = vec![0f32; 48000];
    signal[0] = 0.125;
    p.assets.get_mut("tone").unwrap().samples = Arc::from(signal);
    p.document.mixer.strips[0].mute = false;
    let mut bus = studio::mix_graph::Bus::new("bus".into(), "Monitor bus".into());
    bus.inserts.push(delay("bus-delay", 11));
    p.document.mix_routing.buses.push(bus);
    p.document.mix_routing.track_outputs.insert(
        p.document.tracks[0].id.clone(),
        studio::mix_graph::Destination::Bus("bus".into()),
    );
    p.document
        .mix_routing
        .master
        .inserts
        .push(delay("master-delay", 17));
    p.document.mixer.taps.push(soniclaw::mixer::Tap::PostFader);
    p.document.mixer.strips[0].sends.push(1.);
    p.document.auxiliaries.push(studio::Auxiliary {
        inserts: vec![delay("aux-delay", 23)],
        return_gain: 0.25,
    });
    for (i, source) in [Source::Bus("bus".into()), Source::Aux(0), Source::Main]
        .into_iter()
        .enumerate()
    {
        p.document.cues[0].sends.push(Send {
            id: format!("path-{i}"),
            source,
            gain: 1.,
            pan: 0.,
        });
    }
    let mut prepared = p.document.compile(&p.assets, &mut factory)?;
    assert_eq!(prepared.latency_frames(), 40);
    assert_eq!(prepared.cue_latencies()[0].1, 40);
    let bank = prepared.output_bus().unwrap().clone();
    let mut data = vec![0.; 256];
    prepared.process(
        &mut AudioBuffer::new(&mut data, 2)?,
        &ProcessContext {
            events: &[],
            transport: Transport {
                playing: true,
                ..Default::default()
            },
        },
    );
    let expected = 0.125 + 0.125 * 0.5f32.sqrt() * 2.5;
    assert!((bank.sample(40, 2) - expected).abs() < 1e-6);
    for f in 0..40 {
        assert_eq!(bank.sample(f, 2), 0.);
    }
    let mut p = fixture()?;
    p.document
        .automation
        .lanes
        .push(soniclaw::automation::Lane {
            parameter: "cue.cue-0.gain".into(),
            enabled: true,
            mode: soniclaw::automation::Mode::Read,
            points: vec![
                soniclaw::automation::Point {
                    beat: 0.,
                    value: 0.25,
                    curve: soniclaw::automation::Curve::Step,
                },
                soniclaw::automation::Point {
                    beat: 64. / 24000.,
                    value: 0.,
                    curve: soniclaw::automation::Curve::Step,
                },
            ],
        });
    let render = |size: usize| -> Result<Vec<f32>, ProjectError> {
        let mut prepared = p.document.compile(&p.assets, &mut studio::Builtins)?;
        let bank = prepared.output_bus().unwrap().clone();
        let mut data = vec![0.; size * 2];
        let mut result = vec![];
        for start in (0..128).step_by(size) {
            prepared.process(
                &mut AudioBuffer::new(&mut data, 2)?,
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: start as u64,
                        playing: true,
                        ..Default::default()
                    },
                },
            );
            for f in 0..size {
                result.push(bank.sample(f, 2));
            }
        }
        Ok(result)
    };
    let a = render(128)?;
    let b = render(64)?;
    assert_eq!(a, b);
    assert!(a[..64].iter().all(|v| (*v - 0.2).abs() < 1e-6));
    assert!(a[127] < a[63]);
    Ok(())
}
fn exercise(root: &Path) -> Result<serde_json::Value, ProjectError> {
    std::fs::create_dir(root)?;
    timing_checks()?;
    let p = fixture()?;
    let input = Arc::new(ChannelBus::new(4, 128)?);
    let mut raw = vec![0.; 512];
    for f in raw.as_chunks_mut::<4>().0 {
        f.copy_from_slice(&[0.1, 0.2, 0.3, 0.4]);
    }
    input.write(&raw)?;
    let setup = studio::inputs::Setup {
        bus: input.clone(),
        routes: Default::default(),
    };
    let mut prepared =
        p.document
            .compile_inputs_at(&p.assets, &p.disk, &mut studio::Builtins, 0, &setup)?;
    let bank = prepared.output_bus().unwrap().clone();
    let mut main = vec![0.; 256];
    let measured = allocation_counter::measure(|| {
        prepared.process(
            &mut AudioBuffer::new(&mut main, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport {
                    playing: true,
                    ..Default::default()
                },
            },
        )
    });
    assert_eq!(measured.count_total, 0);
    assert!(main.iter().all(|v| v.abs() < 1e-6));
    for f in 0..128 {
        assert!((bank.sample(f, 2) - 0.2).abs() < 1e-6);
        assert_eq!(bank.sample(f, 4), 0.);
        assert!((bank.sample(f, 6) - 0.3).abs() < 1e-6);
        assert!((bank.sample(f, 8) - 0.25).abs() < 1e-6);
        assert!((bank.sample(f, 9) - 0.3).abs() < 1e-6);
    }
    let pairs: Vec<_> = p.document.cues.iter().map(|c| c.outputs).collect();
    let mapping = OutputMap::new(12, [4, 5], &pairs)?;
    let mut hardware = vec![0.; 128 * 12];
    let routed = allocation_counter::measure(|| {
        mapping
            .render(&main, Some(&bank), &mut hardware, true)
            .unwrap();
    });
    assert_eq!(routed.count_total, 0);
    assert!((hardware[10] - 0.2).abs() < 1e-6);
    assert!((hardware[6] - 0.3).abs() < 1e-6);
    assert_eq!(hardware[8], 0.);
    assert_eq!(hardware[9], 0.);
    assert!(OutputMap::new(12, [0, 1], &pairs).is_err());
    let selector = DeviceSelector {
        name: "Virtual multichannel".into(),
        occurrence: 0,
    };
    let inventory = OutputInventory {
        host: "Virtual".into(),
        hosts: vec!["Virtual".into()],
        default_output: Some(selector.clone()),
        outputs: vec![OutputDevice {
            selector,
            default_rate: 48000,
            default_channels: 2,
            configs: [2, 8, 12]
                .into_iter()
                .map(|channels| OutputRange {
                    channels,
                    min_rate: 48000,
                    max_rate: 48000,
                    buffer_range: None,
                    format: OutputFormat::F32,
                })
                .collect(),
        }],
        ..Default::default()
    };
    let preferences = DevicePreferences {
        sample_rate: Some(48000),
        duplex: Some(DuplexRoute {
            input: None,
            use_default_input: false,
            output_channels: [4, 5],
            cue_outputs: pairs,
            ..Default::default()
        }),
        ..Default::default()
    };
    let plan = negotiate_output(&preferences, &inventory).map_err(|e| e.to_string())?;
    assert_eq!(plan.hardware_channels, 12);
    assert_eq!(plan.spec().channels, 2);
    let mut absent = p.document.compile(&p.assets, &mut studio::Builtins)?;
    let absent_bank = absent.output_bus().unwrap().clone();
    absent.process(
        &mut AudioBuffer::new(&mut main, 2)?,
        &ProcessContext {
            events: &[],
            transport: Transport {
                playing: true,
                ..Default::default()
            },
        },
    );
    assert_eq!(absent_bank.sample(0, 6), 0.);
    let another = p.document.compile(&p.assets, &mut studio::Builtins)?;
    let (mut controller, _processor) = studio::live_pair(another, 0, 1)?;
    let candidate = p.document.compile(&p.assets, &mut studio::Builtins)?;
    assert!(controller.replace(candidate).is_err());
    studio::sessions::save(&p, &root.join("song"))?;
    let reopened = Project::open(&root.join("song"))?;
    assert_eq!(reopened.document.cues.len(), 4);
    let mut changed = p.clone();
    let mut draft = studio::mix_graph::Draft::new(&changed)?;
    draft.document.cues[0].gain = 0.5;
    let mut history = studio::history::History::new(Default::default())?;
    history.edit(&mut changed, "Cue balance", |p| {
        drop(draft.apply(p, &mut studio::Builtins)?);
        Ok(())
    })?;
    history.undo(&mut changed)?;
    assert_eq!(changed.document.cues[0].gain, 1.);
    history.redo(&mut changed)?;
    let track = p.document.tracks[0].id.clone();
    let settings = studio::recording::Settings {
        arms: vec![studio::recording::Arm {
            track,
            source: studio::recording::Source::DryInput,
            monitor_input: false,
            channels: vec![0],
        }],
        count_in_beats: 1,
        ..Default::default()
    };
    let mut recording = studio::recording::prepare_low_latency_with_inputs(
        &p,
        &mut studio::Builtins,
        settings,
        &root.join("take"),
        "take",
        Some(input.clone()),
    )?;
    let output = recording.controller.output_bus().unwrap().clone();
    let mut data = vec![0.; 256];
    let mut crossing = false;
    for block in 0..190 {
        for (f, pair) in raw.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pair[2] = f as f32 / 256.;
        }
        input.write(&raw)?;
        data.fill(0.);
        recording.processor.process(
            &mut AudioBuffer::new(&mut data, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        if block == 187 {
            for f in 0..128 {
                assert!(
                    (output.sample(f, 6) - f as f32 / 256.).abs() < 1e-6,
                    "count-in split lost frame {f}"
                );
            }
            crossing = true;
        }
    }
    assert!(crossing);
    drop(recording.processor);
    recording.recorder.stop()?;
    input.write(&vec![0.; 512])?;
    let mut click = p.clone();
    click.document.cues[0].sends = vec![Send {
        id: "main".into(),
        source: Source::Main,
        gain: 1.,
        pan: 0.,
    }];
    click
        .document
        .mix_routing
        .master
        .inserts
        .push(delay("click-reference-delay", 17));
    click.document.cues[0].metronome = true;
    click.document.cues[0].gain = 0.5;
    click.document.cues[0].pan = 1.;
    let settings = studio::recording::Settings {
        arms: vec![studio::recording::Arm {
            track: click.document.tracks[0].id.clone(),
            source: studio::recording::Source::DryInput,
            monitor_input: false,
            channels: vec![0],
        }],
        count_in_beats: 1,
        ..Default::default()
    };
    let mut rec = studio::recording::prepare_low_latency_with_inputs(
        &click,
        &mut factory,
        settings,
        &root.join("click-take"),
        "click-take",
        Some(input),
    )?;
    let bank = rec.controller.output_bus().unwrap().clone();
    data.fill(0.);
    rec.processor.process(
        &mut AudioBuffer::new(&mut data, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    assert!(data.iter().any(|v| v.abs() > 0.001));
    for f in 0..128 {
        assert_eq!(bank.sample(f, 2), 0.);
        assert!(
            (bank.sample(f, 3)
                - if f >= 17 {
                    data[(f - 17) * 2 + 1] * 0.5
                } else {
                    0.
                })
            .abs()
                < 1e-6
        );
        assert_eq!(bank.sample(f, 6), 0.);
    }
    for _ in 0..190 {
        data.fill(0.);
        rec.processor.process(
            &mut AudioBuffer::new(&mut data, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    drop(rec.processor);
    rec.recorder.stop()?;

    Ok(
        serde_json::json!({"independent_cues":4,"main_muted_while_pre_and_raw_cues_play":true,"physical_channels":12,"highest_mapped_port":11,"processing_channels":2,"post_tap_obeys_track_mute":true,"cue_mix_samples":[0.2,0.,0.3,0.25,0.3],"unmapped_channels_zero":true,"overlap_rejected":true,"cue_callback_allocations":measured.count_total,"mapping_callback_allocations":routed.count_total,"missing_input_is_silent":true,"live_cue_graph_replacement_rejected":true,"count_in_boundary_preserves_cue_subranges":true,"bus_aux_main_cue_alignment_sample":40,"internal_automation_partition_invariant":true,"cue_click_obeys_enabled_gain_balance":true,"cue_click_alignment_frames":17,"portable_cues_and_history":true,"hardware_opened":false}),
    )
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-cues NEW_DIRECTORY")?,
    );
    let report = exercise(&root)?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_cue_workflow() {
        let root = std::env::temp_dir().join(workspace::identity("cues").unwrap());
        exercise(&root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
