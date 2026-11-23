use soniclaw::{
    stream::clips::DiskAssets,
    studio::{
        project::Project,
        tempo_cache::{prepare, Marker, Mode, Recipe},
    },
};
use std::sync::atomic::AtomicBool;
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: tempo-stream NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let source = root.join("source.wav");
    let mut w = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44100,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..44100 {
        let x = (i as f64 * 440. * std::f64::consts::TAU / 44100.).sin() as f32 * 0.2;
        w.write_sample(x)?;
        w.write_sample(x * 0.25)?;
    }
    w.finalize()?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    d.tracks[0].audio.clear();
    d.timeline.tempo[0].bpm = 90.;
    let recipe = Recipe {
        schema: 1,
        mode: Mode::PreservePitch,
        markers: vec![
            Marker {
                source_frame: 0,
                beat: 0.,
            },
            Marker {
                source_frame: 44100,
                beat: 2.,
            },
        ],
    };
    let cache = root.join("cache");
    let binding = prepare(
        &source,
        "original",
        &recipe,
        &d.timeline,
        48000,
        &cache,
        &AtomicBool::new(false),
    )?;
    assert_eq!(binding.frames, 64000);
    let mut project = Project::new(
        "Tempo-aware loop".into(),
        d,
        a,
        DiskAssets::from([("original".into(), source)]),
    )?;
    project.apply_warp(
        "bass",
        "warped",
        "tempo-cache",
        binding,
        &cache.join("audio.wav"),
    )?;
    project.save_new(&root.join("project"))?;
    let loaded = Project::open(&root.join("project"))?;
    assert_eq!(loaded.warps.len(), 1);
    let pcm = hound::WavReader::open(cache.join("audio.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    let mono: Vec<_> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| {
            assert_eq!(f[1], f[0] * 0.25);
            f[0]
        })
        .collect();
    let middle = &mono[4096..mono.len() - 4096];
    let hz = middle
        .windows(2)
        .filter(|w| w[0] <= 0. && w[1] > 0.)
        .count() as f64
        * 48000.
        / middle.len() as f64;
    assert!((hz - 440.).abs() < 5.);
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"input_rate":44100,"output_rate":48000,"target_bpm":90,"frames":mono.len(),"measured_tone_hz":hz,"stereo_linked":true,"portable_reload":true,"callback_processing":"prepared WAV playback; conversion is control/worker-only","hardware_tested":false}),
        )?,
    )?;
    Ok(())
}
