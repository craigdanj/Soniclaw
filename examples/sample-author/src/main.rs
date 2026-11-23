use soniclaw::{authoring::*, plugin::PluginDescriptor, prelude::*};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-sample-author NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut sources = Vec::new();
    for (note, rr) in [(69, 1), (69, 2), (72, 1)] {
        let path = root.join(format!("tone_n{note}_v1-127_rr{rr}.wav"));
        let hz = 440f32 * 2f32.powf((note as f32 - 69.) / 12.);
        let pcm: Vec<_> = (0..24000)
            .map(|i| (i as f32 * hz * std::f32::consts::TAU / 48000.).sin() * 0.15)
            .collect();
        soniclaw::io::write_wav(&path, 48000, 1, &pcm)?;
        sources.push(Source::new(path));
    }
    let estimate = estimate_root(&soniclaw::io::read_wav(&sources[0].path)?)?;
    assert_eq!(estimate.midi_note, 69);
    let export_path = root.join("instrument");
    let report = export(&sources, &export_path, "org.soniclaw.authoring", 0.8)?;
    let (state, asset_root) = soniclaw::sampling::import_sfz(
        &export_path.join("instrument.sfz"),
        "org.soniclaw.authoring",
    )?;
    assert_eq!(state.zones.len(), 3);
    assert_eq!(state.zones[0].group, state.zones[1].group);
    let mut plugin = soniclaw::project::create_plugin(
        PluginDescriptor {
            id: "org.soniclaw.authoring",
            name: "Authored",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: true,
            accepts_midi: true,
        },
        state,
        asset_root,
    )?;
    plugin.activate(ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 256,
    })?;
    let mut output = vec![0.; 48000 * 2];
    let note = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 69,
            velocity: 100,
        },
    }];
    for (i, block) in output.chunks_mut(512).enumerate() {
        plugin.process(block, if i == 0 { &note } else { &[] })?;
    }
    let peak = output.iter().map(|v| v.abs()).fold(0f32, f32::max);
    assert!(peak > 0.01);
    soniclaw::io::write_wav(root.join("preview.wav"), 48000, 2, &output)?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"estimate":estimate,"export":report,"preview_peak":peak,"sfz_regions":3}),
        )?,
    )?;
    println!("Authored three regions, restored SFZ round robins and rendered preview.wav");
    Ok(())
}
