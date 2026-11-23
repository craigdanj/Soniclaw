use soniclaw::{
    prelude::*,
    record::{comp::Recipe, multitrack::*},
    stream::clips::DiskAssets,
    studio::{
        project::{Comp, Project, ProjectError},
        *,
    },
    ui,
};
fn main() -> Result<(), ProjectError> {
    let directory = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: soniclaw-loop-comp NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&directory)?;
    let take_dir = directory.join("original-take");
    let (mut recorder, mut tap) = pair(Config {
        sample_rate: 48000,
        tracks: vec![TrackSpec {
            id: "guitar".into(),
            channels: 1,
            latency_frames: 0,
        }],
        pre_roll_frames: 0,
        queue_packets: 64,
        max_block_frames: 256,
    })?;
    recorder.start(
        &take_dir,
        TakePlan {
            id: "loop-recording".into(),
            label: "Four passes".into(),
            punch: None,
        },
    )?;
    let levels = [0.1, 0.25, 0.5, 0.75, 0.1];
    let mut callback_allocations = 0;
    for block in 0..9 {
        let sample = block * 128;
        let pcm: Vec<_> = (sample..sample + 128).map(|s| levels[s / 256]).collect();
        let markers: Vec<_> = (sample..sample + 128)
            .filter(|s| s % 256 == 0)
            .map(|s| LoopBoundary {
                sample: s as u64,
                source_frame: 12000,
                kind: BoundaryKind::Loop,
            })
            .collect();
        let measured = allocation_counter::measure(|| {
            tap.capture_with_loops(sample as u64, &[&pcm], &markers)
                .unwrap()
        });
        callback_allocations += measured.count_total;
    }
    let take = recorder.stop()?;
    assert!(take.complete);
    let original = std::fs::read(take_dir.join("guitar.wav"))?;
    let mut recipe = Recipe::new(&take)?;
    assert_eq!(recipe.passes.len(), 4);
    recipe.choose(&take, 128, 256, 2)?;
    recipe.crossfade_frames = 16;
    recipe.render_new(&take_dir, "guitar", &directory.join("comp.wav"))?;
    let comp = hound::WavReader::open(directory.join("comp.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(comp.len(), 256);
    assert!((comp[0] - 0.1).abs() < 1e-6);
    assert!((comp[255] - 0.5).abs() < 1e-6);
    std::fs::write(directory.join("recipe.json"), recipe.to_json(&take)?)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    for strip in &mut d.mixer.strips {
        strip.mute = true;
    }
    let mut project = Project::new("Comped performance".into(), d, a, DiskAssets::new())?;
    project.takes.push(take_dir.clone());
    project.apply_comp(
        "guitar-comp",
        Comp {
            take: 0,
            stem: "guitar".into(),
            recipe: recipe.clone(),
        },
    )?;
    project.save_new(&directory.join("portable"))?;
    let loaded = Project::open(&directory.join("portable"))?;
    assert_eq!(loaded.comps.len(), 1);
    let (mut c, mut p) = live_pair(
        loaded
            .document
            .compile_streamed(&loaded.assets, &loaded.disk, &mut Builtins)?,
        0,
        32,
    )?;
    c.set_playing(true)?;
    let mut audio = vec![0.; 12256 * 2];
    let context = ProcessContext {
        events: &[],
        transport: Default::default(),
    };
    let measured = allocation_counter::measure(|| {
        for b in audio.chunks_mut(512) {
            p.process(&mut AudioBuffer::new(b, 2).unwrap(), &context);
        }
    });
    callback_allocations += measured.count_total;
    assert_eq!(callback_allocations, 0);
    assert_eq!(c.status().failures, 0);
    let peak = audio.iter().map(|s| s.abs()).fold(0f32, f32::max);
    assert!((peak - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    assert_eq!(original, std::fs::read(take_dir.join("guitar.wav"))?);
    let ctx = ui::egui::Context::default();
    ctx.enable_accesskit();
    let mut editor = ui::comp::CompEditor::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        editor.show(ui, &take, &mut recipe).unwrap();
    });
    output.textures_delta.clear();
    assert!(output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .any(|(_, n)| n.label() == Some("Use selected pass in range")));
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"passes":4,"comp_sections":2,"loop_frames":256,"crossfade_frames":16,"callback_allocations":callback_allocations,"original_unchanged":true,"portable_comp_reload":true,"live_playback_peak":peak,"headless_take_lanes":true,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
