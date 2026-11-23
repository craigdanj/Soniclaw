use super::*;
use soniclaw_dsp::stream_resample::{SincTable, StreamResampler};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut stream =
        StreamResampler::new(Arc::new(SincTable::new()), 1, 4096, 44100. / 48000., 0.0003)?;
    let input: Vec<f32> = (0..22050)
        .map(|i| 0.45 * (std::f32::consts::TAU * 440. * i as f32 / 44100.).sin())
        .collect();
    let mut w = hound::WavWriter::create(
        out.join("varispeed.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut buffer = [0.; 512];
    let mut count = 0;
    let mut allocations = 0;
    for (i, chunk) in input.chunks(256).enumerate() {
        let mut n = 0;
        allocations += allocation_counter::measure(|| {
            stream.set_increment(0.92 + i as f64 * 0.006).unwrap();
            assert_eq!(stream.push(chunk).unwrap(), chunk.len());
            n = stream.pull(&mut buffer).unwrap();
        })
        .count_total;
        count += n;
        for x in &buffer[..n] {
            assert!(x.is_finite());
            w.write_sample(*x)?;
        }
    }
    stream.finish();
    while !stream.is_drained() {
        let n = stream.pull(&mut buffer)?;
        count += n;
        for x in &buffer[..n] {
            w.write_sample(*x)?;
        }
    }
    w.finalize()?;
    assert_eq!(allocations, 0);
    Ok(
        serde_json::json!({"input_frames":input.len(),"rendered_frames":count,"callback_allocations":allocations,"lookahead":stream.lookahead_source_frames()}),
    )
}
