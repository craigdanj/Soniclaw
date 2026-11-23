//! Contract fixtures for controls intended for use on the prepared audio owner.
use soniclaw_core::*;
#[test]
fn prepared_control_commands_do_not_allocate_or_drop_assets() {
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 128,
        channels: 1,
    };
    let mut string = crate::physical::WaveguideString::new(Default::default()).unwrap();
    string.prepare(spec).unwrap();
    let mut pitch = crate::pitch_quality::Quality::Voice.pitch(2.).unwrap();
    pitch.prepare(spec).unwrap();
    let mut amp = crate::amp_models::Amp::new(Default::default()).unwrap();
    amp.prepare(spec).unwrap();
    let mut dynamics = crate::dynamics::Dynamics::new(Default::default()).unwrap();
    dynamics.prepare(spec).unwrap();
    let count = allocation_counter::measure(|| {
        string.pluck(0.5, 0.3, 42).unwrap();
        string
            .set_config(crate::physical::StringConfig {
                frequency: 330.,
                ..Default::default()
            })
            .unwrap();
        pitch
            .set_formants(Some(crate::streaming::FormantSettings {
                amount: 0.5,
                ..Default::default()
            }))
            .unwrap();
        pitch.set_formants(None).unwrap();
        pitch.set_formants(Some(Default::default())).unwrap();
        amp.set_controls(crate::amp_models::Controls {
            drive_db: 24.,
            ..Default::default()
        })
        .unwrap();
        dynamics
            .set_controls(crate::dynamics::Controls {
                threshold_db: -24.,
                ..Default::default()
            })
            .unwrap();
        assert!(dynamics
            .set_controls(crate::dynamics::Controls {
                ratio: f32::NAN,
                ..Default::default()
            })
            .is_err());
        assert_eq!(dynamics.config().controls.threshold_db, -24.);
        string.reset();
        pitch.reset();
        amp.reset();
        dynamics.reset();
    })
    .count_total;
    assert_eq!(count, 0);
}
