use super::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    for (name, freq) in [("soft", 440.), ("hard", 880.)] {
        let mut w = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for i in 0..8192 {
            w.write_sample(0.45 * (std::f32::consts::TAU * freq * i as f32 / 48000.).sin())?;
        }
        w.finalize()?;
    }
    let path = out.join("articulations.sfz");
    std::fs::write(&path,"<global> sw_default=24\n<region> sample=soft.wav key=60 sw_last=24 ampeg_release=0.01\n<region> sample=hard.wav key=60 sw_last=25 ampeg_release=0.2\n")?;
    let render = |sampler: &mut soniclaw_stream::DiskSampler<4>, events: &[Event]| {
        let mut pcm = [0.; 128];
        let n = allocation_counter::measure(|| {
            sampler.process(
                &mut AudioBuffer::new(&mut pcm, 1).unwrap(),
                &ProcessContext {
                    events,
                    ..context(0)
                },
            )
        })
        .count_total;
        assert_eq!(n, 0);
        pcm
    };
    let on = |note| Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        },
    };
    let off = Event {
        frame: 0,
        message: MidiMessage::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        },
    };
    let mut signatures = Vec::new();
    let mut pages = 0;
    let mut budget = 0;
    for (name, switch) in [("soft-release", 24), ("hard-release", 25)] {
        let instrument = soniclaw::sampling::import_streamed_sfz(&path)?;
        let (worker, mut sampler) = instrument.open::<4>(2 * 1024 * 1024)?;
        sampler.set_envelope(0., 40.)?;
        sampler.set_quality(soniclaw_dsp::resample::ResampleQuality::Linear);
        sampler.prepare(spec())?;
        sampler.reset();
        assert_eq!(sampler.articulation(0), Some(24));
        let mut output = if switch == 24 {
            render(&mut sampler, &[on(60)])
        } else {
            render(&mut sampler, &[on(switch), on(60)])
        }
        .to_vec();
        assert!(worker.wait_ready(1, std::time::Duration::from_secs(2)));
        for _ in 0..8 {
            output.extend(render(&mut sampler, &[]));
        }
        assert_eq!(sampler.articulation(0), Some(switch));
        assert!(output[1024..].iter().any(|x| x.abs() > 0.1));
        output.extend(render(&mut sampler, &[off]));
        for _ in 0..3 {
            output.extend(render(&mut sampler, &[]));
        }
        let held = sampler.active_voices();
        assert_eq!(held, usize::from(switch == 25));
        signatures.push((switch, held));
        let mut w = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in output {
            w.write_sample(x)?;
        }
        w.finalize()?;
        pages += worker
            .stats
            .pages_read
            .load(std::sync::atomic::Ordering::Relaxed);
        budget = worker.pcm_cache_bytes;
        assert_eq!(
            worker
                .stats
                .underrun_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }
    assert!(pages >= 2);
    Ok(
        serde_json::json!({"default_switch": 24, "release_active_voices_after_512_frames": signatures, "preload_budget_bytes": budget, "worker_pages_read": pages, "callback_allocations": 0}),
    )
}
