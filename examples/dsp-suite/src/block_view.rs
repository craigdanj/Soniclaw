use super::*;
use soniclaw_dsp::block_dsp::{Block, BlockMut, Layout};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let planar: Vec<_> = (0..256).map(|i| (i as f32 * 0.11).sin()).collect();
    let mut interleaved = [0.; 256];
    let input = Block::new(&planar, 2, Layout::Planar)?.sub_block(16..112, 0..2)?;
    let mut output = BlockMut::new(&mut interleaved, 2, Layout::Interleaved)?;
    let m = allocation_counter::measure(|| {
        let mut region = output.sub_block_mut(8..104, 0..2).unwrap();
        region.copy_from(input).unwrap();
        for frame in 0..region.as_view().frames() {
            for sample in region.frame_mut(frame).unwrap() {
                *sample *= 0.5;
            }
        }
    });
    assert_eq!(m.count_total, 0);
    assert_eq!(&interleaved[..16], &[0.; 16]);
    let path = out.join("borrowed-region.wav");
    let mut wav = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for sample in interleaved {
        wav.write_sample(sample)?;
    }
    wav.finalize()?;
    Ok(
        serde_json::json!({ "audio":"borrowed-region.wav", "source_frames":[16,112], "destination_frames":[8,104], "channels":2, "callback_allocations":m.count_total }),
    )
}
