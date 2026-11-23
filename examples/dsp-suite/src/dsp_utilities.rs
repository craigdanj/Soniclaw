use super::*;
use soniclaw_dsp::{
    ballistics::{EnvelopeFollower, EnvelopeMode},
    block_dsp::{Block, BlockMath, BlockMut, ClampRange, Layout},
    denormals::with_denormals_flushed,
};

struct Follow(EnvelopeFollower);
impl AudioProcessor for Follow {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        with_denormals_flushed(|| self.0.process(b).expect("matching block"));
    }
    fn reset(&mut self) {
        self.0.reset();
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let math = BlockMath::best();
    let envelope: Vec<_> = (0..48000)
        .map(|i| if (6000..18000).contains(&i) { 1. } else { 0. })
        .collect();
    let mut tone: Vec<_> = (0..48000)
        .map(|i| (std::f32::consts::TAU * i as f32 * 440. / 48000.).cos())
        .collect();
    let gate = Block::new(&envelope, 1, Layout::Interleaved)?;
    let measured = allocation_counter::measure(|| {
        math.multiply(
            gate,
            BlockMut::new(&mut tone, 1, Layout::Interleaved).unwrap(),
        )
        .unwrap();
        math.clamp(
            BlockMut::new(&mut tone, 1, Layout::Interleaved).unwrap(),
            ClampRange::new(-0.8, 0.8).unwrap(),
        );
    });
    assert_eq!(measured.count_total, 0);
    let (peak, rms) = math.peak_rms(Block::new(&tone, 1, Layout::Interleaved)?);
    assert_eq!(peak, 0.8);
    let mut cases = Vec::new();
    for (mode, label) in [(EnvelopeMode::Peak, "peak"), (EnvelopeMode::Rms, "rms")] {
        let follower = EnvelopeFollower::new(spec(), mode, 10., 80.)?;
        let (report, _) = render(out, label, Box::new(Follow(follower)), tone.clone())?;
        cases.push(report);
    }
    Ok(
        serde_json::json!({"backend":format!("{:?}", math.backend()),"peak":peak,"rms":rms,"block_math_allocations":0,"cases":cases}),
    )
}
