use soniclaw_core::*;
use soniclaw_dsp::{
    creative_filters::{NonlinearConfig, NonlinearSvf},
    modulation::{compiled::*, LfoShape},
    Oscillator, Waveform,
};
use std::{path::PathBuf, time::Instant};
fn config(rate: Rate) -> Config {
    Config {
        schema: 1,
        sources: vec![Source {
            id: "lfo".into(),
            kind: SourceKind::Lfo {
                shape: LfoShape::Sine,
                hz: 4.,
                beats: None,
            },
        }],
        destinations: vec![Destination {
            id: "cutoff".into(),
            base: 0.5,
            smoothing_ms: 5.,
        }],
        routes: vec![Route {
            source: "lfo".into(),
            destination: "cutoff".into(),
            depth: 0.45,
            transform: Transform::Direct,
        }],
        rate,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("NEW_DIRECTORY")?);
    std::fs::create_dir(&out)?;
    let mut reports = vec![];
    for (name, rate) in [
        ("audio", Rate::Audio),
        ("control64", Rate::Control { quantum: 64 }),
    ] {
        let mut m = CompiledMatrix::new(config(rate), 48000., 128)?;
        let index = m.destination_index("cutoff")?;
        let mut filter = NonlinearSvf::new(NonlinearConfig {
            resonance: 0.6,
            drive: 2.,
            ..Default::default()
        })?;
        filter.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        })?;
        let mut oscillator = Oscillator::default();
        let mut output = Vec::with_capacity(32768);
        let mut allocations = 0;
        let mut worst = 0u128;
        for _ in 0..256 {
            let mut data = [0.; 128];
            let start = Instant::now();
            allocations += allocation_counter::measure(|| {
                m.render(128, 120., true, &[]).unwrap();
                let controls = m.buffer(index).unwrap();
                for (i, x) in data.iter_mut().enumerate() {
                    let cutoff = 80. * 125f32.powf(controls[i]);
                    filter
                        .set(NonlinearConfig {
                            cutoff_hz: cutoff,
                            resonance: 0.6,
                            drive: 2.,
                            ..Default::default()
                        })
                        .unwrap();
                    *x = 0.2 * oscillator.next_sample(110., 48000., Waveform::Saw);
                    filter.process(
                        &mut AudioBuffer::new(std::slice::from_mut(x), 1).unwrap(),
                        &ProcessContext {
                            events: &[],
                            transport: Transport::default(),
                        },
                    );
                }
            })
            .count_total;
            worst = worst.max(start.elapsed().as_nanos());
            output.extend_from_slice(&data);
        }
        assert_eq!(allocations, 0);
        let expected = if name == "audio" { 32768 } else { 512 };
        assert_eq!(m.stats().source_evaluations, expected);
        assert_eq!(m.stats().route_evaluations, expected);
        assert!(output.iter().all(|x| x.is_finite()));
        let file = format!("{name}-filter.wav");
        let mut w = hound::WavWriter::create(
            out.join(&file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in output {
            w.write_sample(x)?
        }
        w.finalize()?;
        reports.push(serde_json::json!({"mode":name,"stats":m.stats(),"callback_allocations":allocations,"worst_observed_callback_ns":worst,"timing_is_not_a_real_time_guarantee":true,"audio":file}));
    }
    let mut c = config(Rate::Audio);
    c.sources[0].kind = SourceKind::External;
    c.destinations[0].smoothing_ms = 0.;
    let mut matrix = CompiledMatrix::new(c, 48000., 128)?;
    let envelope: Vec<f32> = (0..128).map(|i| i as f32 / 127.).collect();
    matrix.render(
        128,
        120.,
        true,
        &[Input {
            source: 0,
            values: &envelope,
        }],
    )?;
    assert!((matrix.buffer(0)?[127] - 0.95).abs() < 1e-5);
    let saved = serde_json::to_string(matrix.config())?;
    let reopened = CompiledMatrix::new(serde_json::from_str(&saved)?, 48000., 128)?;
    assert_eq!(reopened.config().destinations[0].base, 0.5);
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"cases":reports,"external_envelope_binding":true,"config_roundtrip":true}),
        )?,
    )?;
    Ok(())
}
