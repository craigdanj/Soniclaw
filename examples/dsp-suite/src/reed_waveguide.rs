use super::*;
use soniclaw_dsp::reed_waveguide::{Controls, ReedVoice};

struct Phrase {
    voice: ReedVoice,
    controls: Controls,
}
impl AudioProcessor for Phrase {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.voice.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let block = ctx.transport.sample_position as usize;
        for (i, sample) in b.samples_mut().iter_mut().enumerate() {
            match block + i {
                1200 => {
                    self.controls.breath = 0.78;
                }
                25_000 => {
                    self.controls.frequency_hz *= 1.5;
                    self.controls.breath = 0.84;
                }
                46_000 => {
                    self.controls.breath = 0.;
                }
                _ => {
                    *sample = self.voice.next_sample();
                    continue;
                }
            }
            self.voice
                .set_controls(self.controls)
                .expect("valid scheduled controls");
            *sample = self.voice.next_sample();
        }
    }
    fn reset(&mut self) {
        self.voice.reset();
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut reports = Vec::new();
    for (name, slope, damping) in [("soft-reed", 0.27, 3600.), ("bright-reed", 0.38, 8200.)] {
        let controls = Controls {
            reed_slope: slope,
            damping_hz: damping,
            ..Default::default()
        };
        let (report, pcm) = render(
            out,
            name,
            Box::new(Phrase {
                voice: ReedVoice::new(controls)?,
                controls,
            }),
            vec![0.; 72_000],
        )?;
        assert!(pcm[..1200].iter().all(|&x| x == 0.));
        assert!(pcm[63000..].iter().all(|x| x.abs() < 0.0004));
        reports.push(report);
    }
    Ok(serde_json::json!({"cases":reports,"note_hz":[220,330],"sample_rate":48000}))
}
