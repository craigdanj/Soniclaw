use soniclaw_core::*;
use soniclaw_transport::*;
use soniclaw_vst3::{musical::*, Instance};
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args_os().collect();
    let bundle = std::path::Path::new(
        args.get(1)
            .ok_or("usage: soniclaw-musical-host fixture.vst3 NEW-DIRECTORY")?,
    );
    let directory = std::path::Path::new(args.get(2).ok_or("missing new report directory")?);
    std::fs::create_dir(directory)?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 256,
        channels: 2,
    };
    // SAFETY: user explicitly supplied the trusted independent protocol fixture bundle.
    let mut owner = unsafe { Instance::load(bundle, None, spec) }?;
    if !owner.info()?.name.contains("Host Protocol") {
        return Err("requires independent host protocol fixture".into());
    }
    let timeline = Timeline {
        tempo: vec![
            TempoPoint {
                beat: 0.,
                bpm: 120.,
                transition: Transition::Linear,
            },
            TempoPoint {
                beat: 4.,
                bpm: 240.,
                transition: Transition::Step,
            },
        ],
        meters: vec![MeterPoint {
            beat: 0.,
            numerator: 3,
            denominator: 4,
        }],
        loop_range: Some([0., 0.02]),
        ..Default::default()
    };
    let map = TempoMap::compile(&timeline)?;
    let loop_length = map.sample_at(0.02, 48000.)?;
    let mut processor = owner.start_buffered_musical(Config {
        timeline,
        recording: true,
    })?;
    let mut input = owner.take_native_input().ok_or("no native producer")?;
    let ingress = allocation_counter::measure(|| {
        input.try_sysex(96, &[0xf0, 0x7d, 1, 0xf7]).unwrap();
        input.try_program_change(128, 0, 1).unwrap();
    });
    let target = NoteTarget {
        note_id: 42,
        channel: 0,
        key: 60,
    };
    let notes = [
        Event {
            frame: 7,
            message: MidiMessage::NoteOnId {
                channel: 0,
                note: 60,
                velocity: 0.8,
                note_id: 42,
            },
        },
        Event {
            frame: 64,
            message: MidiMessage::NoteExpression {
                target,
                expression: NoteExpression::Volume,
                value: 1.,
            },
        },
        Event {
            frame: 80,
            message: MidiMessage::NoteExpression {
                target,
                expression: NoteExpression::Tuning,
                value: 12.,
            },
        },
        Event {
            frame: 88,
            message: MidiMessage::NoteExpression {
                target,
                expression: NoteExpression::Pressure,
                value: 0.7,
            },
        },
        Event {
            frame: 200,
            message: MidiMessage::NoteOffId { target },
        },
    ];
    let result = std::thread::scope(|scope| {
        let audio = scope.spawn(move || {
            let mut pcm = vec![0.; 32 * 512];
            let mut allocations = 0;
            for block in 0..32 {
                let data = &mut pcm[block * 512..(block + 1) * 512];
                data.fill(1.);
                let points = if block == 0 {
                    vec![
                        ParameterPoint {
                            frame: 0,
                            id: 9,
                            value: 0.2,
                        },
                        ParameterPoint {
                            frame: 128,
                            id: 9,
                            value: 0.8,
                        },
                    ]
                } else {
                    vec![]
                };
                let ctx = ProcessContext {
                    events: if block == 0 { &notes } else { &[] },
                    transport: Transport {
                        sample_position: block as u64 * 256,
                        tempo: 120.,
                        playing: true,
                    },
                };
                let mut failure = None;
                let m = allocation_counter::measure(|| {
                    failure = processor
                        .process_automated(&mut AudioBuffer::new(data, 2).unwrap(), &ctx, &points)
                        .err();
                });
                allocations += m.count_total;
                assert!(failure.is_none());
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            (pcm, allocations)
        });
        while !audio.is_finished() {
            owner.poll()?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        audio
            .join()
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "audio worker panicked".into()
            })
    })?;
    owner.stop()?;
    let params = owner.parameters()?;
    let value = |id| {
        params
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.value)
            .unwrap_or(-1.)
    };
    let expected_sample = (32 * 256 - 1) % loop_length;
    let beat = map.beat_at_sample(expected_sample, 48000.)?;
    let bpm = map.position(beat)?.bpm;
    let events = owner.drain_timed_output();
    let tuning=events.iter().any(|e|matches!(e.event.data,PluginEventData::NoteExpressionValue{note_id:42,type_id:2,value}if(value-0.55).abs()<1e-8));
    let volume=events.iter().any(|e|matches!(e.event.data,PluginEventData::NoteExpressionValue{note_id:42,type_id:0,value}if(value-0.25).abs()<1e-8));
    let pressure=events.iter().any(|e|matches!(e.event.data,PluginEventData::PolyPressure{note_id:42,pressure,..}if(pressure-0.7).abs()<1e-6));
    let sysex = events.iter().any(
        |e| matches!(&e.event.data,PluginEventData::Data{bytes,..}if bytes==&[0xf0,0x7d,1,0xf7]),
    );
    let stats = owner.stats();
    if result.1 != 0
        || ingress.count_total != 0
        || stats.process_errors != 0
        || stats.input_drops != 0
        || stats.output_drops != 0
        || stats.underruns != 0
        || !tuning
        || !volume
        || !pressure
        || !sysex
    {
        return Err(format!("musical event/real-time checks failed: stats={stats:?} tuning={tuning} volume={volume} pressure={pressure} sysex={sysex}").into());
    }
    if (value(600) * 1000. - bpm).abs() > 1e-7
        || (value(601) * 1024. - beat).abs() > 1e-7
        || value(603) != 3. / 64.
        || value(604) != 4. / 64.
        || value(607) != 7. / 16.
        || (value(10) - 1. / 127.).abs() > 1e-8
    {
        return Err(format!("native musical snapshot incorrect: {:?}", params).into());
    }
    if result.0[2048..2304].iter().any(|x| (*x - 0.2).abs() > 1e-6)
        || result.0[2304..].iter().any(|x| (*x - 0.8).abs() > 1e-6)
    {
        return Err("automation sample offsets incorrect".into());
    }
    let report = serde_json::json!({"native_fixture":owner.info()?.name,"callback_allocations":result.1,"ingress_allocations":ingress.count_total,"stats":stats,"tempo":value(600)*1000.,"ppq":value(601)*1024.,"expected_sample":expected_sample,"meter":[3,4],"loop_end":value(606)*1024.,"recording_playing_cycle_flags":value(607)*16.,"tuning_echo":tuning,"volume_echo":volume,"pressure_echo":pressure,"sysex_echo":sysex,"program":value(10),"automation_split_frame":128,"events":events.len()});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
