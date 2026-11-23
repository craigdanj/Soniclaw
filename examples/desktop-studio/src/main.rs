#[cfg(feature = "gui")]
mod gui;
use soniclaw::{
    prelude::*,
    record::multitrack,
    stream::clips::DiskAssets,
    studio::{project::*, *},
};
use std::path::Path;
fn demonstration(directory: &Path) -> Result<(), ProjectError> {
    std::fs::create_dir(directory)?;
    let (mut document, assets) = soniclaw_studio_demo::fixture();
    document.timeline.loop_range = None;
    document.tracks[1].midi_input = false;
    let mut project = Project::new(
        "Soniclaw Reference Studio".into(),
        document,
        assets,
        DiskAssets::new(),
    )?;
    project.save_new(&directory.join("original"))?;
    let (mut control, mut audio) = live_pair(
        project.document.compile(&project.assets, &mut Builtins)?,
        0,
        64,
    )?;
    let mut pcm = [0.; 512];
    let context = ProcessContext {
        events: &[],
        transport: Default::default(),
    };
    audio.process(&mut AudioBuffer::new(&mut pcm, 2)?, &context);
    let sources = vec![
        ("bass".into(), RecordingSource::Track("bass".into())),
        ("lead".into(), RecordingSource::Track("lead".into())),
        ("master".into(), RecordingSource::Master),
    ];
    let (mut recorder, tap) = multitrack::pair(multitrack::Config {
        sample_rate: 48000,
        tracks: control.recording_specs(&sources)?,
        pre_roll_frames: 256,
        queue_packets: 512,
        max_block_frames: 256,
    })?;
    control.attach_multitrack(
        tap,
        &sources.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(),
    )?;
    control.set_playing(true)?;
    for _ in 0..4 {
        audio.process(&mut AudioBuffer::new(&mut pcm, 2)?, &context);
    }
    let take_directory = directory.join("recorded-take");
    recorder.start(
        &take_directory,
        multitrack::TakePlan {
            id: "studio-take".into(),
            label: "Reference studio stems".into(),
            punch: None,
        },
    )?;
    let allocations = allocation_counter::measure(|| {
        for _ in 0..32 {
            audio.process(&mut AudioBuffer::new(&mut pcm, 2).unwrap(), &context);
        }
    });
    assert_eq!(allocations.count_total, 0);
    let take = recorder.stop()?;
    assert!(take.complete && take.missing_frames == 0);
    control.detach_multitrack()?;
    audio.process(&mut AudioBuffer::new(&mut pcm, 2)?, &context);
    control.collect_recorders();
    project.document = control.snapshot()?;
    project.takes.push(take_directory);
    drop(audio);
    drop(control);
    project.freeze(
        "lead",
        &mut Builtins,
        &directory.join("freeze-cache"),
        0,
        96000,
        24000,
    )?;
    project.save_new(&directory.join("saved"))?;
    let mut reopened = Project::open(&directory.join("saved"))?;
    assert_eq!(reopened.takes.len(), 1);
    assert_eq!(reopened.freezes.len(), 1);
    reopened.unfreeze("lead")?;
    assert!(reopened.document.tracks[1].instrument.is_some());
    let (mut c, mut p) = live_pair(
        reopened
            .document
            .compile_streamed(&reopened.assets, &reopened.disk, &mut Builtins)?,
        0,
        64,
    )?;
    c.set_playing(true)?;
    let mut output = vec![0.; 48000 * 2];
    for b in output.chunks_mut(512) {
        p.process(&mut AudioBuffer::new(b, 2)?, &context);
    }
    let peak = output.iter().map(|s| s.abs()).fold(0., f32::max);
    assert!(peak > 0.01);
    soniclaw::io::write_wav(directory.join("reopened.wav"), 48000, 2, &output)
        .map_err(|e| e.to_string())?;
    reopened.save_new(&directory.join("restored"))?;
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"recorded_frames":take.frames,"callback_allocations":allocations.count_total,"portable_take_reload":true,"portable_freeze_reload":true,"instrument_restored":true,"reopened_peak":peak,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<(), ProjectError> {
    let args: Vec<_> = std::env::args().collect();
    #[cfg(feature = "gui")]
    if args.get(1).map(String::as_str) == Some("--native-scan-child") {
        let format = match args.get(2).map(String::as_str) {
            Some("clap") => soniclaw_host::formats::PluginFormat::Clap,
            Some("vst3") => soniclaw_host::formats::PluginFormat::Vst3,
            _ => return Err("unknown scan format".into()),
        };
        // SAFETY: this internal helper is invoked only for explicitly trusted scans by the parent.
        return unsafe {
            soniclaw_host::studio::registry::child(
                format,
                Path::new(args.get(3).ok_or("module required")?),
                Path::new(args.get(4).ok_or("output required")?),
            )
        };
    }
    match args.get(1).map(String::as_str){Some("--demo")=>demonstration(Path::new(args.get(2).ok_or("--demo needs new directory")?)),Some("--gui")=>{
#[cfg(feature="gui")]{gui::run(&args[2..]).map_err(|e|e.to_string().into())}
#[cfg(not(feature="gui"))]{Err("rebuild with --features gui".into())}},_=>Err("usage: soniclaw-desktop-studio --demo NEW_DIRECTORY | --gui PROJECT_FOLDER [TRUSTED_NATIVE_PATH ...]".into())}
}
