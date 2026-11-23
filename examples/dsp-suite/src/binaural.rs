use super::*;
use soniclaw_dsp::binaural::{Binaural, Direction, HrtfSet};

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let dataset = match args {
        [] => HrtfSet::from_bytes(
            include_bytes!("../assets/MIT_KEMAR_normal_pinna.sofa"),
            48_000,
        )?,
        [path] => HrtfSet::open(path, 48_000)?,
        _ => return Err("expected at most one SOFA path".into()),
    };
    let from = Direction {
        azimuth: 75.,
        elevation: 0.,
        distance_metres: 1.4,
    };
    let (mut control, mut render) = Binaural::prepare(&dataset, from, 128, 512)?;
    let frames = 49_152;
    let mut source = vec![0.; frames];
    let mut seed = 9u32;
    for (i, x) in source.iter_mut().enumerate() {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let envelope = if i % 8192 < 6400 { 0.15 } else { 0. };
        *x = envelope * (seed as f32 / u32::MAX as f32 - 0.5);
    }
    let mut stereo = vec![0.; frames * 2];
    for (block, (x, y)) in source.chunks(128).zip(stereo.chunks_mut(256)).enumerate() {
        if block == 192 {
            control.set_direction(
                &dataset,
                Direction {
                    azimuth: -75.,
                    ..from
                },
            )?;
        }
        let alloc = allocation_counter::measure(|| {
            render
                .process_mono(x, &mut AudioBuffer::new(y, 2).unwrap())
                .unwrap()
        });
        assert_eq!(alloc.count_total, 0);
        control.collect_retired();
    }
    let rms = |range: std::ops::Range<usize>| {
        let mut power = [0f64; 2];
        for &[left, right] in stereo.as_chunks::<2>().0[range].iter() {
            power[0] += f64::from(left).powi(2);
            power[1] += f64::from(right).powi(2);
        }
        (power[0] / power[1]).sqrt()
    };
    let side = [rms(1024..19_456), rms(28_672..47_104)];
    if args.is_empty() {
        assert!(side[0] > 1.2 && side[1] < 1. / 1.2, "ratios {side:?}");
    }
    if !side.iter().all(|x| x.is_finite()) {
        return Err("selected directions returned a silent ear".into());
    }
    let mut writer = hound::WavWriter::create(
        out.join("binaural-left-to-right.wav"),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for value in stereo {
        writer.write_sample(value)?;
    }
    writer.finalize()?;
    Ok(
        serde_json::json!({"measurements":dataset.measurements(),"left_over_right_rms":side,
        "callback_allocations":0,"algorithmic_latency_frames":render.latency_frames()}),
    )
}
