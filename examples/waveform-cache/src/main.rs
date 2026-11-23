use soniclaw_waveform::*;
use std::path::PathBuf;
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-waveform-cache-demo NEW-DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let audio = path.join("minute-stereo.wav");
    let frames = 48000 * 60;
    let mut writer = hound::WavWriter::create(
        &audio,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..frames {
        let t = i as f32 / 48000.;
        writer.write_sample((t * std::f32::consts::TAU * 220.).sin() * 0.8)?;
        writer.write_sample((t * std::f32::consts::TAU * 311.).sin() * 0.4)?;
    }
    writer.finalize()?;
    let cache = path.join("cache");
    let first = load_or_build(&audio, &cache, Default::default(), || false, |_| {})?;
    let second = load_or_build(&audio, &cache, Default::default(), || false, |_| {})?;
    let mut peaks = [Peak::default(); 800];
    second.thumbnail.query(0, 0, frames, &mut peaks)?;
    if first.cache_hit
        || !second.cache_hit
        || first.thumbnail.frames() != frames
        || first.thumbnail.memory_bytes() >= frames as usize * 2 * 4 / 16
    {
        return Err("thumbnail cache/memory check failed".into());
    }
    let mut loader = Loader::new(cache.clone())?;
    loader.request(audio.clone(), Default::default())?;
    let ticket = loader.request(audio.clone(), Default::default())?;
    let start = std::time::Instant::now();
    let async_hit = loop {
        if let Some(done) = loader.poll() {
            if done.ticket != ticket {
                return Err("stale thumbnail result".into());
            }
            break done.result?.cache_hit;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            return Err("background thumbnail timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let detail = read_detail(&audio, 480000, 128, Backend::Wav)?;
    let report = serde_json::json!({"frames":frames,"channels":2,"duration_seconds":60,"source_pcm_bytes":frames*2*4,"peak_memory_bytes":first.thumbnail.memory_bytes(),"levels":first.thumbnail.level_count(),"first_cache_hit":first.cache_hit,"second_cache_hit":second.cache_hit,"latest_async_cache_hit":async_hit,"source_sha256":first.thumbnail.source_sha256(),"detail_samples":detail.len(),"peak_columns":&peaks[..]});
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("PASS sixty-second streaming/cache/latest-request/detail validation");
    Ok(())
}
