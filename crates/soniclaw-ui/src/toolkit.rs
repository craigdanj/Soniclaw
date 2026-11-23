//! Theme tokens and reusable audio controls. All APIs are UI/control-thread operations.
use egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use soniclaw_core::Parameter;
use std::sync::{Arc, OnceLock};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub schema: u32,
    pub name: String,
    pub dark: bool,
    pub background: [u8; 4],
    pub surface: [u8; 4],
    pub control: [u8; 4],
    pub accent: [u8; 4],
    pub text: [u8; 4],
    pub muted: [u8; 4],
    pub danger: [u8; 4],
    pub spacing: f32,
    pub corner_radius: u8,
    pub knob_size: f32,
    pub fader_height: f32,
}
impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}
impl Theme {
    pub fn dark() -> Self {
        Self {
            schema: 1,
            name: "Carbon".into(),
            dark: true,
            background: [20, 25, 35, 255],
            surface: [27, 35, 47, 255],
            control: [43, 53, 68, 255],
            accent: [67, 216, 172, 255],
            text: [231, 237, 246, 255],
            muted: [156, 168, 187, 255],
            danger: [249, 99, 110, 255],
            spacing: 10.,
            corner_radius: 6,
            knob_size: 72.,
            fader_height: 170.,
        }
    }
    pub fn light() -> Self {
        Self {
            name: "Studio paper".into(),
            dark: false,
            background: [244, 242, 236, 255],
            surface: [255, 254, 250, 255],
            control: [219, 223, 225, 255],
            accent: [0, 115, 104, 255],
            text: [32, 42, 53, 255],
            muted: [93, 108, 123, 255],
            danger: [177, 41, 57, 255],
            ..Self::dark()
        }
    }
    pub fn amber() -> Self {
        Self {
            name: "Amber".into(),
            background: [27, 24, 22, 255],
            surface: [38, 33, 29, 255],
            control: [65, 55, 44, 255],
            accent: [247, 181, 85, 255],
            text: [247, 236, 219, 255],
            muted: [181, 164, 143, 255],
            ..Self::dark()
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != 1
            || self.name.is_empty()
            || self.name.len() > 64
            || self.name.contains('\0')
            || !self.spacing.is_finite()
            || !(4.0..=32.0).contains(&self.spacing)
            || !self.knob_size.is_finite()
            || !(40.0..=160.0).contains(&self.knob_size)
            || !self.fader_height.is_finite()
            || !(80.0..=480.0).contains(&self.fader_height)
            || self.corner_radius > 32
        {
            return Err("invalid theme schema/name/metrics");
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, &'static str> {
        if text.len() > 8192 {
            return Err("theme JSON exceeds8KiB");
        }
        let theme: Self = serde_json::from_str(text).map_err(|_| "invalid theme JSON")?;
        theme.validate()?;
        Ok(theme)
    }
    pub fn apply(&self, context: &egui::Context) -> Result<(), &'static str> {
        self.validate()?;
        let color = |v: [u8; 4]| Color32::from_rgba_unmultiplied(v[0], v[1], v[2], v[3]);
        let mut style = (*context.global_style()).clone();
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.window_corner_radius = egui::CornerRadius::same(self.corner_radius);
        visuals.menu_corner_radius = egui::CornerRadius::same(self.corner_radius);
        visuals.panel_fill = color(self.background);
        visuals.window_fill = color(self.surface);
        visuals.extreme_bg_color = color(self.background);
        visuals.faint_bg_color = color(self.surface);
        visuals.override_text_color = Some(color(self.text));
        visuals.selection.bg_fill = color(self.accent).gamma_multiply(0.3);
        visuals.selection.stroke = Stroke::new(1., color(self.accent));
        visuals.widgets.inactive.bg_fill = color(self.control);
        visuals.widgets.hovered.bg_fill =
            color(self.control).gamma_multiply(if self.dark { 1.2 } else { 0.92 });
        visuals.widgets.active.bg_fill = color(self.accent).gamma_multiply(0.45);
        style.visuals = visuals;
        style.spacing.item_spacing = Vec2::splat(self.spacing);
        context.set_theme(if self.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        context.set_global_style(style.clone());
        context.set_style_of(
            if self.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            },
            style,
        );
        context
            .data_mut(|d| d.insert_temp(egui::Id::new("soniclaw-theme"), Arc::new(self.clone())));
        Ok(())
    }
}
pub fn current_theme(context: &egui::Context) -> Arc<Theme> {
    static DEFAULT: OnceLock<Arc<Theme>> = OnceLock::new();
    context
        .data(|d| d.get_temp::<Arc<Theme>>(egui::Id::new("soniclaw-theme")))
        .unwrap_or_else(|| DEFAULT.get_or_init(|| Arc::new(Theme::dark())).clone())
}
pub fn theme_color(rgba: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}
pub(crate) fn accent(ui: &egui::Ui) -> Color32 {
    theme_color(current_theme(ui.ctx()).accent)
}
pub(crate) fn surface(ui: &egui::Ui) -> Color32 {
    theme_color(current_theme(ui.ctx()).surface)
}
#[derive(Clone, Copy, Debug)]
pub struct KnobOptions {
    pub diameter: f32,
    pub sensitivity: f32,
    pub modulation: Option<f32>,
    pub bipolar: bool,
}
impl Default for KnobOptions {
    fn default() -> Self {
        Self {
            diameter: 0.,
            sensitivity: 180.,
            modulation: None,
            bipolar: false,
        }
    }
}
#[derive(Clone, Copy)]
struct DragState {
    value: f32,
    delta: f32,
}
fn interaction(ui: &egui::Ui, response: &mut egui::Response, p: &Parameter, sensitivity: f32) {
    let start = ui.id().with(("audio-control-start", p.info().id));
    let before = p.get();
    if response.drag_started() {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                start,
                DragState {
                    value: p.normalized(),
                    delta: 0.,
                },
            )
        });
    }
    if response.dragged() {
        let mut drag = ui
            .ctx()
            .data(|d| d.get_temp::<DragState>(start))
            .unwrap_or(DragState {
                value: p.normalized(),
                delta: 0.,
            });
        let total = response.total_drag_delta().unwrap_or_default().y;
        let fine = ui.input(|i| if i.modifiers.shift { 0.1 } else { 1. });
        drag.value = (drag.value - (total - drag.delta) / sensitivity * fine).clamp(0., 1.);
        drag.delta = total;
        let _ = p.set_normalized(drag.value);
        ui.ctx().data_mut(|d| d.insert_temp(start, drag));
    }
    if response.clicked() {
        response.request_focus();
    }
    if response.double_clicked() {
        let _ = p.set(p.info().default);
    }
    if response.has_focus() {
        let delta = ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowRight) {
                1.
            } else if i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowLeft) {
                -1.
            } else {
                0.
            }
        });
        if delta != 0. {
            let step = if p.step_count() > 0 {
                1. / p.step_count() as f32
            } else {
                0.01
            };
            let _ = p.set_normalized((p.normalized() + delta * step).clamp(0., 1.));
        }
        if ui.input(|i| i.key_pressed(egui::Key::Home)) {
            let _ = p.set_normalized(0.);
        }
        if ui.input(|i| i.key_pressed(egui::Key::End)) {
            let _ = p.set_normalized(1.);
        }
    }
    if p.get() != before {
        response.mark_changed();
    }
    crate::accessibility::parameter_semantics(ui, response, p);
}
fn arc(ui: &egui::Ui, center: Pos2, radius: f32, start: f32, end: f32, stroke: Stroke) {
    let points: Vec<_> = (0..=48)
        .map(|i| center + Vec2::angled(start + (end - start) * i as f32 / 48.) * radius)
        .collect();
    ui.painter().add(egui::Shape::line(points, stroke));
}
/// Normalized-space knob with accumulated drags, fine adjustment, reset, keyboard and a modulation ring.
pub fn audio_knob(ui: &mut egui::Ui, p: &Parameter, options: KnobOptions) -> egui::Response {
    let theme = current_theme(ui.ctx());
    let diameter = if options.diameter.is_finite() && options.diameter > 0. {
        options.diameter.clamp(40., 160.)
    } else {
        theme.knob_size
    };
    let sensitivity = if options.sensitivity.is_finite() {
        options.sensitivity.clamp(20., 1000.)
    } else {
        180.
    };
    ui.push_id(p.info().id, |ui| {
        ui.vertical(|ui| {
            let (rect, mut response) =
                ui.allocate_exact_size(Vec2::splat(diameter), Sense::click_and_drag());
            interaction(ui, &mut response, p, sensitivity);
            let center = rect.center();
            let radius = diameter * 0.34;
            let start = std::f32::consts::PI * 0.75;
            let span = std::f32::consts::PI * 1.5;
            let n = p.normalized();
            ui.painter()
                .circle_filled(center, radius, theme_color(theme.control));
            arc(
                ui,
                center,
                radius + 3.,
                start,
                start + span,
                Stroke::new(3., theme_color(theme.muted).gamma_multiply(0.3)),
            );
            arc(
                ui,
                center,
                radius + 3.,
                start + if options.bipolar { span * 0.5 } else { 0. },
                start + span * n,
                Stroke::new(3., theme_color(theme.accent)),
            );
            if let Some(offset) = options.modulation.filter(|x| x.is_finite()) {
                arc(
                    ui,
                    center,
                    radius + 8.,
                    start + span * n,
                    start + span * (n + offset).clamp(0., 1.),
                    Stroke::new(2., theme_color(theme.accent).gamma_multiply(0.6)),
                );
            }
            let direction = Vec2::angled(start + span * n);
            ui.painter().line_segment(
                [
                    center + direction * radius * 0.35,
                    center + direction * radius * 0.85,
                ],
                Stroke::new(2., theme_color(theme.text)),
            );
            if response.has_focus() {
                ui.painter().circle_stroke(
                    center,
                    radius + 11.,
                    Stroke::new(1., theme_color(theme.accent)),
                );
            }
            ui.label(p.info().name);
            ui.small(p.format_value(p.get()));
            response.on_hover_text(
                "Drag · Shift for fine adjustment · double-click reset · arrows/Home/End",
            )
        })
        .inner
    })
    .inner
}
/// Vertical audio fader; host parameter law and quantization are retained.
pub fn audio_fader(ui: &mut egui::Ui, p: &Parameter, height: f32) -> egui::Response {
    let theme = current_theme(ui.ctx());
    let height = if height.is_finite() && height > 0. {
        height.clamp(80., 480.)
    } else {
        theme.fader_height
    };
    ui.push_id(p.info().id, |ui| {
        ui.vertical(|ui| {
            let (rect, mut response) =
                ui.allocate_exact_size(Vec2::new(48., height), Sense::click_and_drag());
            interaction(ui, &mut response, p, height);
            let x = rect.center().x;
            let top = rect.top() + 12.;
            let bottom = rect.bottom() - 12.;
            let y = bottom - (bottom - top) * p.normalized();
            ui.painter().line_segment(
                [Pos2::new(x, top), Pos2::new(x, bottom)],
                Stroke::new(5., theme_color(theme.control)),
            );
            ui.painter().line_segment(
                [Pos2::new(x, y), Pos2::new(x, bottom)],
                Stroke::new(3., theme_color(theme.accent)),
            );
            ui.painter().rect_filled(
                Rect::from_center_size(Pos2::new(x, y), Vec2::new(32., 14.)),
                3,
                theme_color(theme.text),
            );
            ui.label(p.info().name);
            ui.small(p.format_value(p.get()));
            response
        })
        .inner
    })
    .inner
}
pub fn audio_toggle(ui: &mut egui::Ui, p: &Parameter) -> egui::Response {
    let mut on = p.normalized() >= 0.5;
    let response = ui.toggle_value(&mut on, p.info().name);
    if response.changed() {
        let _ = p.set_normalized(if on { 1. } else { 0. });
    }
    response
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripAxis {
    Horizontal,
    Vertical,
}
/// Prepared texture ownership and UV selection; decode/upload once, reuse for every frame.
pub struct Filmstrip {
    texture: egui::TextureHandle,
    frames: usize,
    axis: StripAxis,
}
impl Filmstrip {
    pub fn new(
        texture: egui::TextureHandle,
        frames: usize,
        axis: StripAxis,
    ) -> Result<Self, &'static str> {
        let size = texture.size();
        let extent = if axis == StripAxis::Horizontal {
            size[0]
        } else {
            size[1]
        };
        if !(2..=1024).contains(&frames) || extent % frames != 0 || extent / frames == 0 {
            return Err("filmstrip frames must evenly divide texture axis");
        }
        Ok(Self {
            texture,
            frames,
            axis,
        })
    }
    pub fn frame_uv(&self, normalized: f32) -> Rect {
        let n = if normalized.is_finite() {
            normalized.clamp(0., 1.)
        } else {
            0.
        };
        let frame = (n * (self.frames - 1) as f32).round();
        let a = frame / self.frames as f32;
        let b = (frame + 1.) / self.frames as f32;
        if self.axis == StripAxis::Horizontal {
            Rect::from_min_max(Pos2::new(a, 0.), Pos2::new(b, 1.))
        } else {
            Rect::from_min_max(Pos2::new(0., a), Pos2::new(1., b))
        }
    }
    pub fn show(&self, ui: &mut egui::Ui, p: &Parameter, size: Vec2) -> egui::Response {
        let size = if size.is_finite() {
            size.clamp(Vec2::splat(24.), Vec2::splat(512.))
        } else {
            Vec2::splat(72.)
        };
        let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        interaction(ui, &mut response, p, 180.);
        ui.painter().image(
            self.texture.id(),
            rect,
            self.frame_uv(p.normalized()),
            Color32::WHITE,
        );
        response.on_hover_text(p.info().name)
    }
}
#[cfg(feature = "svg")]
pub fn svg_texture(
    context: &egui::Context,
    name: &str,
    bytes: &[u8],
    size: [u32; 2],
) -> Result<egui::TextureHandle, String> {
    if bytes.len() > 1024 * 1024
        || size.contains(&0)
        || size.iter().any(|n| *n > 4096)
        || size[0] as u64 * size[1] as u64 > 16 * 1024 * 1024
    {
        return Err("SVG input or raster size exceeds limits".into());
    }
    let tree = resvg::usvg::Tree::from_data_nested(bytes, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size[0], size[1]).ok_or("could not allocate SVG raster")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            size[0] as f32 / tree.size().width(),
            size[1] as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let image = egui::ColorImage::from_rgba_premultiplied(
        [size[0] as usize, size[1] as usize],
        pixmap.data(),
    );
    Ok(context.load_texture(name, image, egui::TextureOptions::LINEAR))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_roundtrip_and_invalid_metrics() {
        for theme in [Theme::dark(), Theme::light(), Theme::amber()] {
            let copy = Theme::from_json(&serde_json::to_string(&theme).unwrap()).unwrap();
            assert_eq!(theme, copy);
            theme.apply(&egui::Context::default()).unwrap();
        }
        let mut bad = Theme::dark();
        bad.spacing = f32::NAN;
        assert!(bad.validate().is_err());
    }
    #[test]
    fn filmstrip_endpoint_uvs_are_exact() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "strip",
            egui::ColorImage::filled([40, 10], Color32::WHITE),
            Default::default(),
        );
        let strip = Filmstrip::new(texture, 4, StripAxis::Horizontal).unwrap();
        assert_eq!(strip.frame_uv(0.).min, Pos2::ZERO);
        assert_eq!(strip.frame_uv(1.).max, Pos2::new(1., 1.));
        assert_eq!(strip.frame_uv(f32::NAN), strip.frame_uv(0.));
    }
    #[cfg(feature = "svg")]
    #[test]
    fn svg_raster_is_real_and_bounded() {
        let ctx = egui::Context::default();
        let texture=svg_texture(&ctx,"icon",br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##,[16,16]).unwrap();
        assert_eq!(texture.size(), [16, 16]);
        let mut delta = ctx.tex_manager().write().take_delta();
        assert!(delta
            .set
            .values()
            .flat_map(|items| items.iter())
            .any(|d| match &d.image {
                egui::ImageData::Color(c) => c.pixels.iter().any(|p| p.r() > 200 && p.g() < 10),
            }));
        delta.clear();
        assert!(svg_texture(&ctx, "bad", b"x", [0, 20]).is_err());
    }
    #[test]
    fn keyboard_and_accumulated_discrete_drags_work() {
        struct Steps;
        impl soniclaw_core::ParameterCodec for Steps {
            fn quantize(&self, v: f32) -> f32 {
                v.round()
            }
            fn format(&self, v: f32) -> String {
                v.to_string()
            }
            fn parse(&self, t: &str) -> Option<f32> {
                t.parse().ok()
            }
            fn step_count(&self) -> u32 {
                2
            }
        }
        let make = || {
            Parameter::with_codec(
                soniclaw_core::ParameterInfo {
                    id: "choice",
                    name: "Choice",
                    min: 0.,
                    max: 2.,
                    default: 0.,
                    unit: "",
                },
                Arc::new(Steps),
            )
            .unwrap()
        };
        let drag = |pieces: usize| {
            let ctx = egui::Context::default();
            let p = make();
            let mut rect = Rect::NOTHING;
            let mut frame = |events: Vec<egui::Event>| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.))),
                        events,
                        ..Default::default()
                    },
                    |root| {
                        egui::CentralPanel::default().show(root, |ui| {
                            rect = audio_knob(ui, &p, Default::default()).rect;
                        });
                    },
                );
                output.textures_delta.clear();
                rect
            };
            let rect = frame(vec![]);
            let pos = rect.center();
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
            for i in 1..=pieces {
                frame(vec![egui::Event::PointerMoved(
                    pos - Vec2::new(0., 48. * i as f32 / pieces as f32),
                )]);
            }
            p.get()
        };
        assert_eq!(drag(1), 1.);
        assert_eq!(drag(24), 1.);
        let ctx = egui::Context::default();
        let p = make();
        let mut id = egui::Id::NULL;
        let mut output = ctx.run_ui(Default::default(), |root| {
            id = audio_knob(root, &p, Default::default()).id;
        });
        output.textures_delta.clear();
        ctx.memory_mut(|m| m.request_focus(id));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |root| {
                audio_knob(root, &p, Default::default());
            },
        );
        output.textures_delta.clear();
        assert_eq!(p.get(), 1.);
    }
}
