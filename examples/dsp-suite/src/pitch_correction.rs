use super::*;
use soniclaw_dsp::{
    advanced::analysis::PitchDetector,
    pitch_correction::{Controls, PitchCorrector, Settings},
};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<_> = (0..48_000)
        .map(|i| 0.25 * (std::f32::consts::TAU * 426. * i as f32 / 48_000.).sin())
        .collect();
    let controls = Controls {
        note_mask: Controls::scale(0, &[0, 2, 4, 5, 7, 9, 11])?,
        ..Controls::default()
    };
    let mut detector = PitchDetector::new(48_000., 4096, 320., 620.)?;
    let mut hz = Vec::new();
    let mut cases = Vec::new();
    for (name, strength) in [("original-aligned", 0.), ("scale-corrected", 1.)] {
        let correction = PitchCorrector::new(
            Settings {
                minimum_hz: 140.,
                ..Settings::default()
            },
            Controls {
                strength,
                ..controls
            },
        )?;
        let (report, rendered) = render(out, name, Box::new(correction), input.clone())?;
        hz.push(
            detector
                .estimate(&rendered[35_000..39_096])
                .ok_or("pitch not found")?
                .hz,
        );
        cases.push(report);
    }
    assert!(
        (hz[0] - 426.).abs() < 1. && (hz[1] - 440.).abs() < 2.,
        "{hz:?}"
    );
    Ok(serde_json::json!({"cases":cases,"measured_hz":hz}))
}
