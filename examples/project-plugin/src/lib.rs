//! Editable project rack exported as CLAP with complete structural state.
use soniclaw::{
    plugin::{PluginDescriptor, PluginInstance},
    state::{Connection, ProcessorKind, ProjectState, RackNode},
};
use soniclaw_clap::{descriptor, Definition};
pub fn default_project() -> ProjectState {
    let mut p = ProjectState::empty("org.soniclaw.project-rack", "Soniclaw Project Rack");
    let nodes = [
        ("eq", ProcessorKind::Eq),
        ("compressor", ProcessorKind::Compressor),
        ("chorus", ProcessorKind::Chorus),
        ("reverb", ProcessorKind::Reverb),
        ("gain", ProcessorKind::Gain),
        ("limiter", ProcessorKind::Limiter),
    ];
    for (i, (id, kind)) in nodes.into_iter().enumerate() {
        p.nodes.push(RackNode {
            custom: None,
            voice_modulation: None,
            asset: None,
            id: id.into(),
            kind,
            settings: Default::default(),
            input_gain: if i == 0 { 1.0 } else { 0.0 },
        });
        if i > 0 {
            p.connections.push(Connection {
                from: nodes[i - 1].0.into(),
                to: id.into(),
                gain: 1.0,
            });
        }
    }
    p.outputs.insert("limiter".into(), 1.0);
    p
}
pub fn create() -> PluginInstance {
    soniclaw::project::create_plugin(
        PluginDescriptor {
            id: "org.soniclaw.project-rack",
            name: "Soniclaw Project Rack",
            vendor: "Soniclaw",
            version: env!("CARGO_PKG_VERSION"),
            is_instrument: false,
            accepts_midi: false,
        },
        default_project(),
        ".",
    )
    .expect("valid factory project")
}
static DESCRIPTOR: soniclaw_clap::ClapDescriptor = descriptor(
    c"org.soniclaw.project-rack",
    c"Soniclaw Project Rack",
    c"Soniclaw",
    c"0.146.0",
    false,
);
soniclaw_clap::export_clap!([Definition {
    descriptor: &DESCRIPTOR,
    create,
    instrument: false,
    tail_seconds: 4.0
}]);
