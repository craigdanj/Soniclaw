//! Reusable comp/take lane editor. Recipes remain drafts until the application commits them.
use egui::{self, Align2, FontId, Pos2, Rect, Sense, Vec2};
use soniclaw_commands::History;
use soniclaw_record::{
    comp::Recipe,
    multitrack::{RecordError, Take},
};
pub struct CompEditor {
    pub selected_pass: u32,
    pub start: u64,
    pub end: u64,
    history: History<Recipe>,
}
impl Default for CompEditor {
    fn default() -> Self {
        Self {
            selected_pass: 0,
            start: 0,
            end: 0,
            history: History::new(16).expect("bounded history"),
        }
    }
}
impl CompEditor {
    pub fn reset(&mut self, recipe: &Recipe) {
        self.selected_pass = recipe.passes[0].id;
        self.start = 0;
        self.end = recipe.loop_frames;
        self.history = History::new(16).expect("bounded history");
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        take: &Take,
        recipe: &mut Recipe,
    ) -> Result<bool, RecordError> {
        recipe.validate(take)?;
        if self.end == 0 {
            self.reset(recipe);
        }
        let mut changed = false;
        let mut error = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("Undo comp"))
                .clicked()
            {
                changed |= self.history.undo(recipe);
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("Redo comp"))
                .clicked()
            {
                changed |= self.history.redo(recipe);
            }
            let mut fade = recipe.crossfade_frames;
            if ui
                .add(
                    egui::DragValue::new(&mut fade)
                        .prefix("Crossfade frames ")
                        .range(0..=4096),
                )
                .changed()
            {
                if let Err(e) = self.history.edit(recipe, |candidate| {
                    candidate.crossfade_frames = fade;
                    candidate.validate(take)
                }) {
                    error = Some(e);
                } else {
                    changed = true;
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        let width = ui.available_width().clamp(120., 8192.);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 44.), Sense::hover());
        for (index, section) in recipe.sections.iter().enumerate() {
            let r = Rect::from_min_max(
                Pos2::new(
                    rect.left() + section.start as f32 / recipe.loop_frames as f32 * width,
                    rect.top(),
                ),
                Pos2::new(
                    rect.left() + section.end as f32 / recipe.loop_frames as f32 * width,
                    rect.bottom(),
                ),
            );
            let response = ui.interact(r, ui.id().with(("comp-section", index)), Sense::click());
            if crate::accessibility::selectable(
                ui,
                &response,
                &format!(
                    "Comp frames {} to {}, pass {}",
                    section.start, section.end, section.pass
                ),
                self.start == section.start && self.end == section.end,
            ) {
                self.start = section.start;
                self.end = section.end;
                self.selected_pass = section.pass;
            }
            ui.painter().rect_filled(
                r,
                2.,
                egui::Color32::from_rgb(40 + (section.pass % 5 * 30) as u8, 120, 140),
            );
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("Pass {}", section.pass),
                FontId::proportional(12.),
                egui::Color32::WHITE,
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("comp-passes")
            .max_height(180.)
            .show_rows(ui, 24., recipe.passes.len(), |ui, range| {
                for index in range {
                    let pass = &recipe.passes[index];
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.selected_pass,
                            pass.id,
                            format!("Pass {}", pass.id),
                        );
                        ui.label(format!(
                            "acquisition {}–{} · {} frames",
                            pass.start_sample,
                            pass.end_sample,
                            pass.end_sample - pass.start_sample
                        ));
                    });
                }
            });
        let mut choose = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.start)
                    .prefix("From frame ")
                    .range(0..=recipe.loop_frames),
            );
            ui.add(
                egui::DragValue::new(&mut self.end)
                    .prefix("To frame ")
                    .range(0..=recipe.loop_frames),
            );
            choose = ui.button("Use selected pass in range").clicked();
        });
        if choose {
            self.history.edit(recipe, |candidate| {
                candidate.choose(take, self.start, self.end, self.selected_pass)
            })?;
            changed = true;
        }
        Ok(changed)
    }
}
