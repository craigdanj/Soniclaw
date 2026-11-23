use soniclaw_core::{Parameter, ParameterInfo};
use soniclaw_ui::{egui, *};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: accessibility-audit NEW_REPORT_JSON")?;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let parameter = Parameter::new(ParameterInfo {
        id: "audit.gain",
        name: "Output gain",
        min: 0.,
        max: 2.,
        default: 1.,
        unit: "x",
    })?;
    let mut out = ctx.run_ui(Default::default(), |ui| {
        audio_knob(ui, &parameter, Default::default());
        ui.push_id("fader", |ui| {
            audio_fader(ui, &parameter, 120.);
        });
    });
    out.textures_delta.clear();
    let tree = out
        .platform_output
        .accesskit_update
        .ok_or("missing semantic tree")?;
    let nodes:Vec<_>=tree.nodes.iter().map(|(_,n)|serde_json::json!({"role":format!("{:?}",n.role()),"label":n.label(),"value":n.value(),"numeric_value":n.numeric_value(),"min":n.min_numeric_value(),"max":n.max_numeric_value(),"increment":n.supports_action(egui::accesskit::Action::Increment),"set_value":n.supports_action(egui::accesskit::Action::SetValue)})).collect();
    assert!(
        nodes
            .iter()
            .filter(|n| n["label"] == "Output gain"
                && n["role"] == "Slider"
                && n["set_value"] == true)
            .count()
            >= 2
    );
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(&nodes)?)?;
    Ok(())
}
