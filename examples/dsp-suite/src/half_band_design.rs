use super::*;
use soniclaw_dsp::half_band_design::*;
struct PassThrough;
impl AudioProcessor for PassThrough {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, _: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {}
    fn reset(&mut self) {}
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let up = FilterSpec {
        transition_width: 0.10,
        attenuation_db: 74.,
    };
    let down = FilterSpec {
        transition_width: 0.065,
        attenuation_db: 86.,
    };
    let mut reports = Vec::new();
    for (label, family) in [
        ("fir", Family::EquirippleFir),
        ("iir", Family::PolyphaseIir),
    ] {
        let designs = [
            HalfBand::design(family, up)?,
            HalfBand::design(family, down)?,
        ];
        let mut engine = CustomOversampler::new(spec(), &[StageSpec { family, up, down }])?;
        let mut samples: Vec<_> = (0..8192).map(|i| 0.6 * (i as f32 * 0.693).sin()).collect();
        let mut allocations = 0;
        for part in samples.chunks_mut(128) {
            let mut buffer = AudioBuffer::new(part, 1)?;
            allocations += allocation_counter::measure(|| {
                engine
                    .process_with(&mut buffer, |audio, s| {
                        assert_eq!(s.sample_rate, 96000.);
                        for sample in audio.samples_mut() {
                            *sample = (2.5 * *sample).tanh();
                        }
                    })
                    .unwrap();
            })
            .count_total;
        }
        assert_eq!(allocations, 0);
        let name = format!("custom-2x-{label}");
        let (mut report, _) = render(out, &name, Box::new(PassThrough), samples)?;
        report.as_object_mut().unwrap().remove("latency_frames");
        report["dc_delay_samples"] = engine.dc_delay_samples().into();
        report["measured_stopbands_db"] =
            serde_json::json!(designs.map(|d| d.measured_attenuation_db()));
        report["oversampling_callback_allocations"] = allocations.into();
        reports.push(report);
    }
    Ok(serde_json::json!({ "cases": reports }))
}
