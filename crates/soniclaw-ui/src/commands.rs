//! Shared command presentation; returned IDs are dispatched by the application.
use soniclaw_commands::Registry;
pub fn command_button(ui: &mut egui::Ui, registry: &Registry, id: &str) -> bool {
    registry.get(id).is_some_and(|c| {
        ui.add_enabled(c.enabled, egui::Button::new(&c.label).selected(c.checked))
            .clicked()
    })
}
pub fn command_menu(ui: &mut egui::Ui, registry: &Registry, category: &str) -> Option<String> {
    let mut selected = None;
    for c in registry.commands().filter(|c| c.category == category) {
        if command_button(ui, registry, &c.id) {
            selected = Some(c.id.clone());
            ui.close();
        }
    }
    selected
}
/// Routes only exact modifiers and suppresses shortcuts while a text widget owns the keyboard.
pub fn command_shortcuts(ctx: &egui::Context, registry: &Registry) -> Option<String> {
    if ctx.egui_wants_keyboard_input() {
        return None;
    }
    let events = ctx.input(|i| i.events.clone());
    for e in events {
        if let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = e
        {
            let k = soniclaw_commands::Shortcut {
                key: key.name().into(),
                control: modifiers.ctrl,
                alt: modifiers.alt,
                shift: modifiers.shift,
                command: modifiers.mac_cmd,
            };
            if let Some(id) = registry.route(&k) {
                ctx.input_mut(|i| {
                    i.consume_key(modifiers, key);
                });
                return Some(id.into());
            }
        }
    }
    None
}
