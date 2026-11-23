use soniclaw::{
    clips::{self, AudioClip, Edit, FadeShape, GainPoint, RegionEdits},
    prelude::*,
    stream::clips::{edit_regions, DiskAssets, DiskClip, StreamedClips},
    studio::project::{Project, ProjectError},
    ui,
};
use std::sync::Arc;
fn render(d: &clips::Document, a: &clips::Assets) -> Result<Vec<f32>, ProjectError> {
    let mut p = d.compile(a, 64, None)?;
    let mut output = vec![0.; 2048];
    let mut at = 0;
    let measured = allocation_counter::measure(|| {
        for block in output.chunks_mut(128) {
            p.render(
                &mut AudioBuffer::new(block, 2).unwrap(),
                Transport {
                    sample_position: at,
                    tempo: 120.,
                    playing: true,
                },
            )
            .unwrap();
            at += 64;
        }
    });
    assert_eq!(measured.count_total, 0);
    Ok(output)
}
fn main() -> Result<(), ProjectError> {
    let path = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: audio-editing NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let samples: Arc<[f32]> = Arc::from(
        (0..64)
            .map(|i| (i as f32 * std::f32::consts::TAU / 64.).sin())
            .collect::<Vec<_>>(),
    );
    let assets = clips::Assets::from([(
        "wave".into(),
        clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: samples.clone(),
        },
    )]);
    let mut doc = clips::Document {
        schema: 1,
        sample_rate: 48000,
        channels: 2,
        timeline: Default::default(),
        audio: vec![AudioClip {
            edits: Some(Box::new(RegionEdits {
                fade_shape: FadeShape::EqualPower,
                ..RegionEdits::new(1024)
            })),
            id: "region".into(),
            asset: "wave".into(),
            track: 0,
            start_frame: 0,
            length_frames: 1024,
            source_offset: 0,
            source_length: 64,
            looping: true,
            gain: 0.7,
            fade_in: 50,
            fade_out: 100,
            enabled: true,
        }],
        midi: vec![],
    };
    doc.apply(Edit::GainEnvelope {
        id: "region".into(),
        points: vec![
            GainPoint {
                frame: 0,
                gain: 0.2,
            },
            GainPoint {
                frame: 512,
                gain: 1.,
            },
            GainPoint {
                frame: 1024,
                gain: 0.3,
            },
        ],
    })?;
    let original = render(&doc, &assets)?;
    doc.apply(Edit::SplitAudio {
        id: "region".into(),
        new_id: "right".into(),
        frame: 373,
    })?;
    assert_eq!(render(&doc, &assets)?, original);
    doc.apply(Edit::JoinAudio {
        left: "region".into(),
        right: "right".into(),
    })?;
    assert_eq!(render(&doc, &assets)?, original);
    doc.apply(Edit::SlipAudio {
        id: "region".into(),
        frames: -13,
    })?;
    let edited = render(&doc, &assets)?;
    assert!(edited
        .iter()
        .zip(&original)
        .any(|(a, b)| (a - b).abs() > 0.01));
    soniclaw::io::write_wav(path.join("edited.wav"), 48000, 2, &edited)
        .map_err(|e| e.to_string())?;
    let mut writer = hound::WavWriter::create(
        path.join("source.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for &sample in samples.iter() {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    let source_bytes = std::fs::read(path.join("source.wav"))?;
    let mut regions = vec![DiskClip::from_audio(doc.audio[0].clone())];
    edit_regions(
        &mut regions,
        Edit::SplitAudio {
            id: "region".into(),
            new_id: "disk-right".into(),
            frame: 373,
        },
        48000,
    )?;
    let disk = DiskAssets::from([("wave".into(), path.join("source.wav"))]);
    let spec = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 64,
    };
    let mut streaming =
        StreamedClips::new(&regions, &disk, spec, 2 * 1024 * 1024).map_err(|e| e.to_string())?;
    assert!(streaming.prime_at(0, std::time::Duration::from_secs(1)));
    let mut output = vec![0.; 2048];
    let mut at = 0;
    let measured = allocation_counter::measure(|| {
        for block in output.chunks_mut(128) {
            streaming.process(
                &mut AudioBuffer::new(block, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: at,
                        tempo: 120.,
                        playing: true,
                    },
                },
            );
            at += 64;
        }
    });
    assert_eq!(measured.count_total, 0);
    let error = edited
        .iter()
        .zip(&output)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert_eq!(error, 0.);
    assert_eq!(source_bytes, std::fs::read(path.join("source.wav"))?);
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    d.tracks[0].audio.clear();
    d.tracks[0].streamed = regions;
    let mut project = Project::new("Region editing".into(), d, a, disk)?;
    let mut draft = project.edit_view()?;
    let mut editor = ui::timeline::TimelineEditor::default();
    editor.selection.clips.insert("region".into());
    let ctx = ui::egui::Context::default();
    ctx.enable_accesskit();
    let mut frame = ctx.run_ui(Default::default(), |ui| {
        editor.audio_tools(ui, &mut draft.document).unwrap();
    });
    frame.textures_delta.clear();
    assert!(frame
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Apply gain envelope")));
    project.replace_edit_view(draft)?;
    project.save_new(&path.join("portable"))?;
    let loaded = Project::open(&path.join("portable"))?;
    assert!(loaded.document.tracks[0]
        .streamed
        .iter()
        .all(|c| c.edits.is_some()));
    std::fs::write(path.join("regions.json"), doc.to_json()?)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"split_sample_exact":true,"join_sample_exact":true,"memory_disk_max_error":error,"source_unchanged":true,"callback_allocations":measured.count_total,"headless_audio_tools":true,"portable_edit_metadata":true,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
