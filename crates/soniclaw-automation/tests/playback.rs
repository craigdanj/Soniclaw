use soniclaw_automation::*;
use soniclaw_core::*;
use std::sync::Arc;
fn document() -> Document {
    Document {
        loop_range: None,
        lanes: vec![Lane {
            parameter: "gain".into(),
            enabled: true,
            mode: Mode::Touch,
            points: vec![
                Point {
                    beat: 0.0,
                    value: 0.0,
                    curve: Curve::Linear,
                },
                Point {
                    beat: 1.0,
                    value: 1.0,
                    curve: Curve::Step,
                },
            ],
        }],
    }
}
struct Scalar(Arc<Parameter>);
impl AudioProcessor for Scalar {
    fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
        Ok(())
    }
    fn reset(&mut self) {}
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for x in b.samples_mut() {
            *x *= self.0.get();
        }
    }
}
fn prepared(doc: Document) -> (AutomatedProcessor, Vec<Gesture>) {
    let p = Arc::new(
        Parameter::new(ParameterInfo {
            id: "gain",
            name: "Gain",
            min: 0.0,
            max: 1.0,
            default: 1.0,
            unit: "",
        })
        .unwrap(),
    );
    let (mut a, g) = AutomatedProcessor::new(Box::new(Scalar(p.clone())), doc, &[p]).unwrap();
    a.prepare(ProcessSpec {
        sample_rate: 8.0,
        max_block_frames: 8,
        channels: 1,
    })
    .unwrap();
    (a, g)
}
fn process(a: &mut AutomatedProcessor, at: u64, tempo: f64, playing: bool, n: usize) -> Vec<f32> {
    let mut data = vec![1.0; n];
    a.process(
        &mut AudioBuffer::new(&mut data, 1).unwrap(),
        &ProcessContext {
            events: &[],
            transport: Transport {
                sample_position: at,
                tempo,
                playing,
            },
        },
    );
    data
}
#[test]
fn curves_and_endpoint_holds() {
    let mut d = document();
    for curve in [
        Curve::Step,
        Curve::Linear,
        Curve::Smooth,
        Curve::Bezier { c1: 0.2, c2: 0.8 },
    ] {
        d.lanes[0].points[0].curve = curve;
        d.validate().unwrap();
        assert_eq!(d.lanes[0].value_at(-1.0), Some(0.0));
        assert_eq!(d.lanes[0].value_at(2.0), Some(1.0));
        assert_eq!(
            d.lanes[0].value_at(0.5),
            Some(if curve == Curve::Step { 0.0 } else { 0.5 })
        );
    }
}
#[test]
fn partition_invariance_loop_and_seek() {
    let mut d = document();
    d.loop_range = Some([0.0, 1.0]);
    let (mut a, _) = prepared(d.clone());
    let full = process(&mut a, 0, 60.0, true, 8);
    let (mut b, _) = prepared(d);
    let mut split = process(&mut b, 0, 60.0, true, 3);
    split.extend(process(&mut b, 3, 60.0, true, 5));
    assert_eq!(full, split);
    assert_eq!(process(&mut b, 8, 60.0, true, 2), vec![0.0, 0.125]);
    assert_eq!(process(&mut b, 4, 60.0, true, 1), vec![0.5]);
}
#[test]
fn tempo_continuity_and_touch_latch() {
    let (mut a, g) = prepared(document());
    assert_eq!(
        process(&mut a, 0, 60.0, true, 4),
        vec![0.0, 0.125, 0.25, 0.375]
    );
    assert_eq!(process(&mut a, 4, 120.0, true, 2), vec![0.5, 0.75]);
    g[0].set(0.2).unwrap();
    assert_eq!(process(&mut a, 6, 120.0, true, 1), vec![0.2]);
    g[0].end();
    assert_eq!(process(&mut a, 7, 120.0, true, 1), vec![1.0]);
    let mut d = document();
    d.lanes[0].mode = Mode::Latch;
    let (mut a, g) = prepared(d);
    g[0].set(0.3).unwrap();
    g[0].end();
    assert_eq!(process(&mut a, 0, 60.0, true, 1), vec![0.3]);
    process(&mut a, 1, 60.0, false, 1);
    assert_eq!(process(&mut a, 2, 60.0, true, 1), vec![0.25]);
}
#[test]
fn recorder_snap_transaction_and_modes() {
    let mut d = document();
    let mut r = Recorder::default();
    r.begin(0.7).unwrap();
    assert!(r.tick(&mut d, 0, 0.37, 0.25).unwrap());
    assert_eq!(d.lanes[0].points[1].beat, 0.25);
    r.end();
    assert!(!r.tick(&mut d, 0, 0.5, 0.25).unwrap());
    let before = d.clone();
    assert!(d.write(0, 0.2, f32::NAN, 0.25).is_err());
    assert_eq!(d, before);
    d.lanes[0].mode = Mode::Latch;
    r.begin(0.8).unwrap();
    r.tick(&mut d, 0, 0.5, 0.25).unwrap();
    r.end();
    assert!(r.tick(&mut d, 0, 0.75, 0.25).unwrap());
    r.stop();
    assert!(!r.tick(&mut d, 0, 1.0, 0.25).unwrap());
}
#[test]
fn bounds_unknown_targets_and_serialization() {
    let mut d = document();
    d.lanes.push(d.lanes[0].clone());
    assert!(d.validate().is_err());
    d.lanes.pop();
    d.lanes[0].parameter = "missing".into();
    assert!(AutomatedProcessor::new(
        Box::new(Scalar(Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.0,
                max: 1.0,
                default: 1.0,
                unit: ""
            })
            .unwrap()
        ))),
        d,
        &[]
    )
    .is_err());
    let d = document();
    assert_eq!(
        d,
        serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap()
    );
}
#[test]
fn callback_has_no_allocations() {
    let (mut a, _) = prepared(document());
    let mut data = [1.0; 8];
    let count = allocation_counter::measure(|| {
        a.process(
            &mut AudioBuffer::new(&mut data, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: 0,
                    tempo: 60.0,
                    playing: true,
                },
            },
        );
    });
    assert_eq!(count.count_total, 0);
}

#[test]
fn extreme_transport_position_saturates_inner_sample_offsets() {
    let (mut processor, _) = prepared(document());
    let result = process(&mut processor, u64::MAX, 60.0, true, 8);
    assert!(result.iter().all(|x| x.is_finite()));
}
