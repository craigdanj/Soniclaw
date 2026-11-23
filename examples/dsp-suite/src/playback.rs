use super::*;
use soniclaw_dsp::{sample_playback::*, Sample};
struct Started(RegionPlayer);
impl AudioProcessor for Started {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.0.prepare(s)?;
        self.0.start()
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        self.0.process(b, c);
    }
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let sample = Arc::new(Sample::new(
        (0..24000)
            .map(|i| {
                let t = i as f32 / 48000.;
                let wave = (1..10)
                    .map(|h| 0.2 / h as f32 * (std::f32::consts::TAU * 220. * h as f32 * t).sin())
                    .sum::<f32>();
                wave * (0.1 + 0.9 * (-t * 4.).exp())
            })
            .collect::<Vec<_>>(),
        1,
        48000.,
        57,
    )?);
    let mut cases = vec![];
    for (name, reverse, mode) in [
        ("forward", false, None),
        ("reverse", true, None),
        ("crossfaded-loop", false, Some(LoopMode::Wrap)),
        ("pingpong-loop", false, Some(LoopMode::PingPong)),
    ] {
        let config = Config {
            schema: 1,
            start: 0,
            end: sample.frames(),
            reverse,
            speed: 0.8,
            gain: 0.8,
            interpolation: Interpolation::SincFast,
            loop_region: mode.map(|mode| Loop {
                start: 4096,
                end: 16384,
                mode,
                crossfade_frames: if mode == LoopMode::Wrap { 256 } else { 0 },
            }),
        };
        cases.push(
            render(
                out,
                name,
                Box::new(Started(RegionPlayer::new(sample.clone(), config)?)),
                vec![0.; 48000],
            )?
            .0,
        );
    }
    let endpoint = match_loop_end(&sample, 4096, 16000, 128, 0)?;
    Ok(serde_json::json!({"cases":cases,"offline_matched_end":endpoint}))
}
