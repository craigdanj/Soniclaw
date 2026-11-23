use soniclaw::{automation::*, plugin::PluginDescriptor, prelude::*, state::*};
fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "test",
        name: "Test",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: false,
        accepts_midi: false,
    }
}
#[test]
fn project_automation_survives_state_and_builds() {
    let mut s = ProjectState::empty("test", "Automation");
    let mut n = RackNode::new("gain", ProcessorKind::Gain);
    n.input_gain = 1.0;
    s.nodes.push(n);
    s.outputs.insert("gain".into(), 1.0);
    let p = soniclaw::project::create_plugin(descriptor(), s.clone(), ".").unwrap();
    let id = p.parameters()[0].info().id.to_owned();
    s.automation = Some(Document {
        loop_range: Some([0.0, 4.0]),
        lanes: vec![Lane {
            parameter: id,
            enabled: true,
            mode: Mode::Read,
            points: vec![Point {
                beat: 0.0,
                value: 0.0,
                curve: Curve::Linear,
            }],
        }],
    });
    let restored = ProjectState::from_json(&s.to_json().unwrap()).unwrap();
    assert_eq!(s, restored);
    let mut p = soniclaw::project::create_plugin(descriptor(), restored, ".").unwrap();
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        channels: 2,
        max_block_frames: 8,
    })
    .unwrap();
    p.set_transport(Transport {
        sample_position: 0,
        tempo: 120.0,
        playing: true,
    })
    .unwrap();
    let mut data = [1.0; 16];
    p.process(&mut data, &[]).unwrap();
    assert_eq!(p.parameters()[0].get(), 0.0);
    assert!(data[0] < 1.0 && data[14] < data[0]);
    s.automation.as_mut().unwrap().lanes[0].parameter = "missing".into();
    assert!(soniclaw::project::create_plugin(descriptor(), s, ".").is_err());
}
