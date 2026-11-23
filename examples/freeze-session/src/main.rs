use soniclaw::{
    stream::clips::DiskAssets,
    studio::{freeze::*, *},
};
fn main() -> Result<(), FreezeError> {
    let directory = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: soniclaw-freeze-session NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&directory)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.tracks[1].midi_input = false;
    d.timeline.loop_range = None;
    let frozen = render(
        &d,
        "lead",
        &a,
        &DiskAssets::new(),
        &mut Builtins,
        &directory.join("lead-cache"),
        0,
        96000,
        24000,
    )?;
    let (playback, catalog) = frozen.apply(&d, &a, &DiskAssets::new())?;
    let _prepared = playback.compile_streamed(&a, &catalog, &mut Builtins)?;
    std::fs::write(directory.join("frozen-project.json"), playback.to_json()?)?;
    std::fs::write(
        directory.join("restored-project.json"),
        frozen.unfreeze(&playback)?.to_json()?,
    )?;
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":frozen.frames,"sha256":frozen.wav_sha256,"original_instrument":d.tracks[1].instrument.is_some(),"frozen_instrument":playback.tracks[1].instrument.is_some(),"cache_reload_verified":Frozen::read(&directory.join("lead-cache"))?.verify().is_ok(),"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
