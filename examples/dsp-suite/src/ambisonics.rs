use super::*;
use soniclaw_dsp::{
    ambisonics::{encode, HeadphoneDecoder, Rotation},
    binaural::HrtfSet,
};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let set = HrtfSet::from_bytes(
        include_bytes!("../assets/MIT_KEMAR_normal_pinna.sofa"),
        48_000,
    )?;
    let mut decoder = HeadphoneDecoder::new(&set, 1.4, 128)?;
    let mut seed = 42u32;
    let frames = 24_576;
    let data = (0..frames)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed as f32 / u32::MAX as f32 - 0.5) * 0.15;
            let yaw = if i < 8192 {
                90.
            } else if i >= 16_384 {
                -90.
            } else {
                90. - 180. * (i - 8192) as f32 / 8192.
            };
            Rotation::new(yaw, 0., 0.)
                .unwrap()
                .apply(encode(x, 0., 0.).unwrap())
        })
        .collect::<Vec<_>>();
    let mut stereo = vec![0.; 2 * frames];
    for (frames, buffer) in data.chunks(128).zip(stereo.chunks_mut(256)) {
        let measured = allocation_counter::measure(|| {
            decoder
                .process(frames, &mut AudioBuffer::new(buffer, 2).unwrap())
                .unwrap()
        });
        assert_eq!(measured.count_total, 0);
    }
    let ratio = |range: std::ops::Range<usize>| {
        let mut sums = [0.; 2];
        for &[l, r] in &stereo.as_chunks::<2>().0[range] {
            sums[0] += f64::from(l).powi(2);
            sums[1] += f64::from(r).powi(2);
        }
        (sums[0] / sums[1]).sqrt()
    };
    let ratios = [ratio(1024..7168), ratio(18_432..23_552)];
    assert!(ratios[0] > 1.2 && ratios[1] < 1. / 1.2, "{ratios:?}");
    let mut ambi = hound::WavWriter::create(
        out.join("rotated-ambix.wav"),
        hound::WavSpec {
            channels: 4,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for x in data.into_iter().flatten() {
        ambi.write_sample(x)?;
    }
    ambi.finalize()?;
    let mut stereo_out = hound::WavWriter::create(
        out.join("ambix-headphones.wav"),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for x in stereo {
        stereo_out.write_sample(x)?;
    }
    stereo_out.finalize()?;
    Ok(
        serde_json::json!({"left_right_rms_ratios":ratios,"sofa_measurements":set.measurements(),"callback_allocations":0,"order":"W,Y,Z,X"}),
    )
}
