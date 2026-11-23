//! AccessKit semantics and actions shared by painted audio controls.
use egui::{
    accesskit::{Action, ActionData},
    *,
};
use soniclaw_core::Parameter;
/// Adds range/value semantics and handles assistive increment/decrement/set-value actions.
/// Numeric values use the parameter's stored units; formatted text uses its codec.
pub fn parameter_semantics(ui: &Ui, response: &mut Response, p: &Parameter) {
    let before = p.get();
    let info = p.info();
    if ui.is_enabled() {
        ui.input(|input| {
            let delta = input.num_accesskit_action_requests(response.id, Action::Increment) as f32
                - input.num_accesskit_action_requests(response.id, Action::Decrement) as f32;
            let step = if p.step_count() > 0 {
                1. / p.step_count() as f32
            } else {
                0.01
            };
            if delta != 0. {
                let _ = p.set_normalized((p.normalized() + delta * step).clamp(0., 1.));
            }
            for request in input.accesskit_action_requests(response.id, Action::SetValue) {
                match &request.data {
                    Some(ActionData::NumericValue(value)) if value.is_finite() => {
                        let _ = p.set((*value as f32).clamp(info.min, info.max));
                    }
                    Some(ActionData::Value(text)) => {
                        if let Some(v) = p.parse_value(text) {
                            let _ = p.set(v);
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    if before != p.get() {
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), p.get() as f64, info.name));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_min_numeric_value(info.min as f64);
        node.set_max_numeric_value(info.max as f64);
        node.set_value(p.format_value(p.get()));
        node.set_numeric_value_step(if p.step_count() > 0 {
            (info.max - info.min) as f64 / p.step_count() as f64
        } else {
            (info.max - info.min) as f64 * 0.01
        });
        if ui.is_enabled() {
            node.add_action(Action::SetValue);
            if p.get() < info.max {
                node.add_action(Action::Increment);
            }
            if p.get() > info.min {
                node.add_action(Action::Decrement);
            }
        }
    });
}
/// Announce a painted, focusable clip/note and map keyboard/accessibility selection.
pub(crate) fn selectable(ui: &Ui, response: &Response, label: &str, selected: bool) -> bool {
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.clicked()
        || response.gained_focus()
        || (ui.is_enabled()
            && response.has_focus()
            && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)))
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::ParameterInfo;
    fn make() -> Parameter {
        Parameter::new(ParameterInfo {
            id: "test.gain",
            name: "Gain",
            min: 0.,
            max: 2.,
            default: 1.,
            unit: "x",
        })
        .unwrap()
    }
    #[test]
    fn semantic_tree_and_real_assistive_actions() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        let p = make();
        let mut id = Id::NULL;
        let mut first = ctx.run_ui(Default::default(), |ui| {
            id = crate::audio_fader(ui, &p, 100.).id;
        });
        first.textures_delta.clear();
        let update = first.platform_output.accesskit_update.unwrap();
        let node = update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Gain"))
            .unwrap();
        assert_eq!(node.1.role(), accesskit::Role::Slider);
        assert_eq!(node.1.numeric_value(), Some(1.));
        assert_eq!(node.1.min_numeric_value(), Some(0.));
        assert_eq!(node.1.max_numeric_value(), Some(2.));
        let node_id = node.0;
        for (action, data, expected) in [
            (Action::Increment, None, 1.02),
            (Action::SetValue, Some(ActionData::NumericValue(1.5)), 1.5),
            (
                Action::SetValue,
                Some(ActionData::Value("0.75".into())),
                0.75,
            ),
        ] {
            let mut output = ctx.run_ui(
                RawInput {
                    events: vec![Event::AccessKitActionRequest(accesskit::ActionRequest {
                        action,
                        target_node: node_id,
                        target_tree: accesskit::TreeId::ROOT,
                        data,
                    })],
                    ..Default::default()
                },
                |ui| {
                    assert_eq!(crate::audio_fader(ui, &p, 100.).id, id);
                },
            );
            output.textures_delta.clear();
            assert!((p.get() - expected).abs() < 1e-6);
        }
        let mut output = ctx.run_ui(
            RawInput {
                events: vec![Event::AccessKitActionRequest(accesskit::ActionRequest {
                    action: Action::Increment,
                    target_node: node_id,
                    target_tree: accesskit::TreeId::ROOT,
                    data: None,
                })],
                ..Default::default()
            },
            |ui| {
                ui.disable();
                assert_eq!(crate::audio_fader(ui, &p, 100.).id, id);
            },
        );
        output.textures_delta.clear();
        assert_eq!(p.get(), 0.75);
    }
}
