//! Hardware-free audible examples and measurements for all eight DSP areas.
use soniclaw::{
    dsp::{
        advanced::{analysis::*, crossover::ThreeWayCrossover, spectral::*},
        AutomatedEffect, EffectKind,
    },
    prelude::*,
};
use std::{fmt::Write, path::PathBuf};
fn sp() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    }
}
fn run(kind: EffectKind, input: &[f32]) -> Result<Vec<f32>, Error> {
    let mut p = AutomatedEffect::new(kind, "fx")?;
    p.prepare(sp())?;
    let mut data = input.to_vec();
    for block in data.chunks_mut(512) {
        p.process(
            &mut AudioBuffer::new(block, 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    Ok(data)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "soniclaw-dsp-lab-output".into()),
    );
    std::fs::create_dir(&out)?;
    let seconds = 3;
    let source: Vec<f32> = (0..48000 * seconds)
        .flat_map(|i| {
            let t = i as f32 / 48000.0;
            let pulse = if i % 24000 < 18000 { 1.0 } else { 0.0 };
            let x = ((std::f32::consts::TAU * 220.0 * t).sin() * 0.2
                + (std::f32::consts::TAU * 880.0 * t).sin() * 0.1
                + (std::f32::consts::TAU * 5000.0 * t).sin() * 0.05)
                * pulse;
            [x, x]
        })
        .collect();
    let save = |name: &str, data: &[f32]| soniclaw::io::write_wav(out.join(name), 48000, 2, data);
    save("source.wav", &source)?;
    for (kind, name) in [
        (EffectKind::StateVariable, "state-variable-filter.wav"),
        (EffectKind::Ladder, "ladder-filter.wav"),
        (EffectKind::FdnReverb, "fdn-reverb.wav"),
        (EffectKind::SpectralGate, "spectral-gate.wav"),
    ] {
        save(name, &run(kind, &source)?)?;
    }
    let mut creative = source.clone();
    for kind in [
        EffectKind::Phaser,
        EffectKind::Flanger,
        EffectKind::Tremolo,
        EffectKind::AutoPan,
        EffectKind::MultiTapDelay,
    ] {
        creative = run(kind, &creative)?;
    }
    save("creative-chain.wav", &creative)?;
    let dynamics_source: Vec<_> = source
        .iter()
        .enumerate()
        .map(|(i, x)| *x * if i / 2 % 24000 < 1200 { 7.0 } else { 2.0 })
        .collect();
    let mut dynamics = run(EffectKind::TransientShaper, &dynamics_source)?;
    dynamics = run(EffectKind::RmsCompressor, &dynamics)?;
    dynamics = run(EffectKind::LookaheadLimiter, &dynamics)?;
    save("dynamics.wav", &dynamics)?;
    let mut split = [
        ThreeWayCrossover::new(48000.0, 300.0, 3000.0)?,
        ThreeWayCrossover::new(48000.0, 300.0, 3000.0)?,
    ];
    let mut stems = [vec![], vec![], vec![]];
    for frame in source.as_chunks::<2>().0 {
        let l = split[0].split(frame[0]);
        let r = split[1].split(frame[1]);
        for band in 0..3 {
            stems[band].extend_from_slice(&[l[band], r[band]]);
        }
    }
    for (i, name) in [
        "crossover-low.wav",
        "crossover-mid.wav",
        "crossover-high.wav",
    ]
    .into_iter()
    .enumerate()
    {
        save(name, &stems[i])?;
    }
    let mut wave = soniclaw_dsp_lab::create_wave();
    wave.activate(sp())?;
    let mut audio = vec![0.0; 48000 * seconds * 2];
    for (block, data) in audio.chunks_mut(512).enumerate() {
        if block == 180 {
            wave.parameters()
                .iter()
                .find(|p| p.info().id == "osc.morph")
                .unwrap()
                .set(0.9)?;
        }
        if block == 360 {
            wave.parameters()
                .iter()
                .find(|p| p.info().id == "osc.pwm")
                .unwrap()
                .set(0.7)?;
        }
        let events = if block == 0 {
            vec![Event {
                frame: 0,
                message: MidiMessage::NoteOnId {
                    channel: 1,
                    note: 60,
                    velocity: 0.8,
                    note_id: 42,
                },
            }]
        } else {
            vec![]
        };
        wave.process(data, &events)?;
    }
    save("wavetable-unison.wav", &audio)?;
    std::fs::write(out.join("wavetable-project.json"), wave.save_state()?)?;
    wave.deactivate();
    let fx = soniclaw_dsp_lab::create_effect();
    std::fs::write(out.join("effects-project.json"), fx.save_state()?)?;
    let tone: Vec<_> = (0..4096)
        .map(|i| 0.1 * (std::f64::consts::TAU * 440.0 * i as f64 / 48000.0).sin() as f32)
        .collect();
    let mut pitch = PitchDetector::new(48000.0, 4096, 50.0, 2000.0)?;
    let estimate = pitch.estimate(&tone).ok_or("pitch estimate failed")?;
    let mut loudness = LoudnessMeter::new(48000, 2, 128)?;
    loudness.process(&dynamics)?;
    let mut peak = TruePeakDetector::new(2)?;
    let mut stereo = StereoMeter::default();
    for f in dynamics.as_chunks::<2>().0 {
        peak.push_frame(f);
        stereo.push(f[0], f[1]);
    }
    for _ in 0..12 {
        peak.push_frame(&[0.0; 2]);
    }
    let mut analyzer = SpectrumAnalyzer::new(4096, WindowKind::Hann)?;
    let spectrum = analyzer.analyze(&tone)?;
    let mut csv = String::from("frequency_hz,magnitude\n");
    for (i, m) in spectrum.iter().enumerate() {
        writeln!(&mut csv, "{},{}", i as f64 * 48000.0 / 4096.0, m)?;
    }
    std::fs::write(out.join("spectrum.csv"), csv)?;
    let report = serde_json::json!({"version":"0.146.0","sample_rate":48000,"seconds":seconds,"pitch_hz":estimate.hz,"pitch_confidence":estimate.confidence,"integrated_lufs":loudness.integrated_lufs(),"momentary_lufs":loudness.momentary_lufs(),"true_peak_dbtp":peak.peak_dbtp(),"stereo_correlation":stereo.correlation(),"lookahead_latency_frames":252,"stft_latency_frames":1024,"hardware_audio_tested":false});
    std::fs::write(
        out.join("analysis.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    println!(
        "DSP lab completed all eight areas in {}\n{}",
        out.display(),
        report
    );
    Ok(())
}
