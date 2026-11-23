use soniclaw::{
    core::*,
    studio::{
        self, media_tools,
        project::{Project, ProjectError},
        stock, workspace, Factory,
    },
};
use std::path::Path;
fn exercise(root: &Path) -> Result<serde_json::Value, ProjectError> {
    std::fs::create_dir(root)?;
    let wav = root.join("owned-tone.wav");
    let mut w = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..24000 {
        w.write_sample((i as f32 * std::f32::consts::TAU * 440. / 24000.).sin() * 0.4)?;
    }
    w.finalize()?;
    let mut p = workspace::blank("Stock tools".into(), 48000, 128)?;
    let asset = media_tools::decode(&wav, 48000)?;
    assert_eq!(asset.frames(), 48000);
    let track = p.document.tracks[0].id.clone();
    stock::install_sample(&mut p, &track, "sample", asset, 60)?;
    let sampler = p.document.tracks[0].instrument.as_ref().unwrap().id.clone();
    media_tools::save_preset(&p, &sampler, "My sample", &root.join("Sample preset"))?;
    let mid = root.join("performance.mid");
    use soniclaw::midi::file::{ClipEvent, MidiClip, TempoChange};
    std::fs::write(
        &mid,
        MidiClip {
            ppqn: 480,
            tempos: vec![TempoChange {
                tick: 0,
                micros_per_quarter: 600000,
            }],
            events: vec![
                ClipEvent {
                    tick: 0,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                },
                ClipEvent {
                    tick: 120,
                    message: MidiMessage::PitchBend {
                        channel: 0,
                        value: 10000,
                    },
                },
                ClipEvent {
                    tick: 240,
                    message: MidiMessage::ControlChange {
                        channel: 0,
                        controller: 64,
                        value: 127,
                    },
                },
                ClipEvent {
                    tick: 480,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                },
                ClipEvent {
                    tick: 720,
                    message: MidiMessage::ControlChange {
                        channel: 0,
                        controller: 64,
                        value: 0,
                    },
                },
            ],
        }
        .write()?,
    )?;
    media_tools::import_midi(&mut p, &mid, &track, 0., true)?;
    assert_eq!(p.document.timeline.tempo[0].bpm, 100.);
    assert_eq!(p.document.tracks[0].midi[0].controls.len(), 3);
    let bad = p.clone();
    let invalid = stock::effect("missing", "bad");
    assert!(invalid.is_err());
    assert_eq!(p.document.to_json()?, bad.document.to_json()?);
    stock::add_effect(&mut p, &track, stock::effect("gain", "gain")?)?;
    let mut built = p
        .document
        .compile(&p.assets, &mut stock::AssetsFactory { assets: &p.assets })?;
    assert_eq!(built.latency_frames(), 0);
    let gain = built
        .parameters()
        .iter()
        .find(|v| v.info().id == "gain.gain")
        .unwrap()
        .clone();
    let ctx = ProcessContext {
        events: &[],
        transport: Transport {
            playing: true,
            ..Default::default()
        },
    };
    let mut pcm = vec![0.; 256];
    built.process(&mut AudioBuffer::new(&mut pcm, 2)?, &ctx);
    let peak = pcm.iter().copied().map(f32::abs).fold(0., f32::max);
    assert!(peak > 0.001);
    gain.set(0.)?;
    for i in 1..10 {
        pcm.fill(0.);
        built.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                transport: Transport {
                    sample_position: i * 128,
                    playing: true,
                    ..Default::default()
                },
                events: &[],
            },
        );
    }
    assert!(pcm.iter().all(|v| v.abs() < 1e-6));
    drop(built);
    p.document.parameter_values.insert("gain.gain".into(), 0.25);
    media_tools::save_preset(&p, "gain", "My gain", &root.join("Gain preset"))?;
    std::fs::remove_file(wav)?;
    let sample_preset = media_tools::load_preset(&root.join("Sample preset"), 48000)?;
    let mut portable = workspace::blank("Portable".into(), 48000, 128)?;
    let target = portable.document.tracks[0].id.clone();
    media_tools::install_preset(&mut portable, &target, sample_preset)?;
    assert_eq!(portable.assets.len(), 1);
    media_tools::install_preset(
        &mut portable,
        &target,
        media_tools::load_preset(&root.join("Gain preset"), 48000)?,
    )?;
    let d = portable.document.tracks[0].inserts[0].id.clone();
    assert_eq!(
        portable.document.parameter_values[&format!("{d}.gain")],
        0.25
    );
    let plan = studio::media::audit(&p)?;
    assert!(!plan.report().opaque_dependencies);
    assert!(plan
        .report()
        .entries
        .iter()
        .any(|e| e.id == "sample" && e.referenced));
    studio::media::consolidate(&p, &plan, &root.join("song"))?;
    let reopened = Project::open(&root.join("song"))?;
    assert!(reopened.assets.contains_key("sample"));
    let entries = media_tools::scan(root, "")?;
    assert!(entries.iter().any(|e| e.kind == media_tools::Kind::Preset));
    assert!(entries.iter().any(|e| e.kind == media_tools::Kind::Midi));
    let spec = p.document.spec();
    for (name, _) in stock::EFFECTS {
        let mut fx = studio::Builtins.create(&stock::effect(name, "test")?)?;
        fx.processor.prepare(spec)?;
        let mut data = vec![0.1; 256];
        fx.processor
            .process(&mut AudioBuffer::new(&mut data, 2)?, &ctx);
        assert!(data.iter().all(|v| v.is_finite()), "{name}");
    }
    let mut latency = studio::Builtins.create(&stock::effect("lookahead-limiter", "limit")?)?;
    latency.processor.prepare(spec)?;
    assert!(latency.processor.latency_frames() > 0);
    let preview_asset = reopened.assets["sample"].clone();
    let mut preview = media_tools::Preview::new(preview_asset);
    preview.prepare(spec)?;
    let mut data = vec![0.; 256];
    preview.process(&mut AudioBuffer::new(&mut data, 2)?, &ctx);
    assert!(data.iter().any(|v| v.abs() > 0.01));
    Ok(
        serde_json::json!({"stock_effects_prepared_and_finite":stock::EFFECTS.len(),"resampled_frames":48000,"midi_controls_imported":3,"file_tempo_bpm":100,"sampler_peak":peak,"live_gain_smoothing_reaches_silence":true,"portable_sampler_after_original_deleted":true,"preset_parameter_namespace_remapped":true,"sampler_retained_by_consolidation":true,"zero_latency_session_frames":0,"lookahead_latency_frames":latency.processor.latency_frames(),"media_scan_and_preview":true,"hardware_opened":false}),
    )
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-tools NEW_DIRECTORY")?,
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
    fn portable_stock_workflow() {
        let root = std::env::temp_dir().join(format!(
            "soniclaw-tools-{}",
            workspace::identity("test").unwrap()
        ));
        let report = exercise(&root).unwrap();
        assert_eq!(report["stock_effects_prepared_and_finite"], 32);
        std::fs::remove_dir_all(root).unwrap();
    }
}
