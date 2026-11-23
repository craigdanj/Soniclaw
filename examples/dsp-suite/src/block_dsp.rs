use super::*;
use soniclaw_dsp::block_dsp::*;
pub fn run(_: &Path) -> Result<serde_json::Value> {
    let m = ChannelMatrix::new(2, 1, vec![0.5, 0.5])?;
    let mut timings = Vec::new();
    for n in [32, 64, 128, 1024, 4096] {
        let x = vec![0.4; n * 2];
        let mut y = vec![0.; n];
        let input = Block::new(&x, 2, Layout::Planar)?;
        let start = std::time::Instant::now();
        let alloc = allocation_counter::measure(|| {
            for _ in 0..1000 {
                m.mix(
                    input,
                    BlockMut::new(std::hint::black_box(&mut y), 1, Layout::Planar).unwrap(),
                )
                .unwrap();
            }
        })
        .count_total;
        assert_eq!(alloc, 0);
        assert!(y.iter().all(|&s| s == 0.4));
        timings.push((n, start.elapsed().as_nanos() / 1000));
    }
    Ok(
        serde_json::json!({"backend":format!("{:?}",m.backend()),"frames_and_ns_per_block":timings,"optimized":!cfg!(debug_assertions)}),
    )
}
