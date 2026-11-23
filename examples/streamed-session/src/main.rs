use soniclaw::{prelude::*, stream::clips::*, studio::*};
use std::{sync::atomic::Ordering, time::Duration};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-streamed-session NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let file = path.join("long-source.wav");
    let mut writer = hound::WavWriter::create(
        &file,
        hound::WavSpec {
            sample_rate: 48000,
            channels: 1,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..48000 * 60 {
        writer
            .write_sample((i as f64 * 110. * std::f64::consts::TAU / 48000.).sin() as f32 * 0.15)?;
    }
    writer.finalize()?;
    let (mut document, _) = soniclaw_studio_demo::fixture();
    document.timeline.loop_range = None;
    document.tracks[0].audio.clear();
    document.tracks[0].streamed.push(DiskClip {
        edits: None,
        id: "long-recording".into(),
        asset: "disk-bass".into(),
        start_frame: 0,
        length_frames: 144000,
        source_offset: 0,
        source_length: 48000 * 60,
        looping: false,
        gain: 1.,
        fade_in: 200,
        fade_out: 200,
        enabled: true,
    });
    let assets = DiskAssets::from([("disk-bass".into(), file.clone())]);
    let prepared =
        document.compile_streamed(&soniclaw::clips::Assets::new(), &assets, &mut Builtins)?;
    let (mut controller, mut processor) = live_pair(prepared, 0, 64)?;
    controller.set_playing(true)?;
    let mut pcm = vec![0.; 144000 * 2];
    let mut allocation_count = 0;
    for chunk in pcm.chunks_mut(512) {
        let alloc = allocation_counter::measure(|| {
            processor.process(
                &mut AudioBuffer::new(chunk, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Default::default(),
                },
            )
        });
        allocation_count += alloc.count_total;
        std::thread::sleep(Duration::from_millis(6));
    }
    assert_eq!(allocation_count, 0);
    assert!(pcm.iter().any(|v| v.abs() > 0.01));
    let statistics:Vec<_>=controller.streaming_stats().iter().map(|s|serde_json::json!({"clip":s.clip_id,"pcm_cache_bytes":s.cache_bytes,"underrun_frames":s.stats.underrun_frames.load(Ordering::Relaxed),"io_errors":s.stats.io_errors.load(Ordering::Relaxed),"request_overflows":s.stats.request_overflows.load(Ordering::Relaxed),"pages_read":s.stats.pages_read.load(Ordering::Relaxed),"stale_pages":s.stats.stale_pages.load(Ordering::Relaxed)})).collect();
    assert!(controller
        .streaming_stats()
        .iter()
        .all(|s| s.stats.io_errors.load(Ordering::Relaxed) == 0));
    soniclaw::io::write_wav(path.join("streamed-session.wav"), 48000, 2, &pcm)?;
    std::fs::write(path.join("session.json"), controller.snapshot()?.to_json()?)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"file_bytes":std::fs::metadata(file)?.len(),"source_frames":48000*60,"rendered_frames":144000,"callback_allocations":allocation_count,"streams":statistics,"paced_worker_test":true,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
