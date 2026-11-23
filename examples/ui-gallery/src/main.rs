//! Native audio-independent gallery, with optional reproducible GPU screenshots.
use soniclaw::{
    params::*,
    prelude::*,
    ui::{self, egui},
};
use std::path::PathBuf;
enum_choices! {pub enum Mode{Clean=>"Clean",Warm=>"Warm",Bright=>"Bright"}}
struct Gallery {
    cutoff: FloatParam,
    gain: FloatParam,
    enabled: BoolParam,
    mode: EnumParam<Mode>,
    frequency: Parameter,
    adsr: AdsrSettings,
    wave: ui::WaveformView,
    samples: Vec<f32>,
    held: [bool; 128],
    last_note: String,
    theme: usize,
    modulation: f32,
    icon: egui::TextureHandle,
    strip: ui::Filmstrip,
    loader: Option<soniclaw::waveform::Loader>,
    thumbnail: Option<std::sync::Arc<soniclaw::waveform::Thumbnail>>,
    thumbnail_view: ui::ThumbnailView,
    cache_hit: bool,
    wave_error: Option<String>,
    waveform_only: bool,
    snapshot: Option<PathBuf>,
    frames: u32,
    requested: bool,
}
fn themes() -> [ui::Theme; 3] {
    [ui::Theme::dark(), ui::Theme::light(), ui::Theme::amber()]
}
fn assets(ctx: &egui::Context, theme: &ui::Theme) -> (egui::TextureHandle, ui::Filmstrip) {
    let rgb = |v: [u8; 4]| format!("#{:02x}{:02x}{:02x}", v[0], v[1], v[2]);
    let accent = rgb(theme.accent);
    let icon = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><path fill="{accent}" d="M8 4 L28 16 L8 28 Z"/></svg>"##
    );
    let icon = ui::svg_texture(ctx, "play", icon.as_bytes(), [32, 32]).unwrap();
    let mut svg =
        String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="936" height="72">"#);
    for i in 0..13 {
        let x = i * 72 + 36;
        let angle = std::f32::consts::PI * (0.75 + 1.5 * i as f32 / 12.);
        let end = [x as f32 + angle.cos() * 22., 36. + angle.sin() * 22.];
        svg.push_str(&format!(r##"<circle cx="{x}" cy="36" r="28" fill="{}" stroke="{accent}" stroke-width="2"/><path d="M{x} 36 L{} {}" stroke="{}" stroke-width="3"/>"##,rgb(theme.control),end[0],end[1],rgb(theme.text)));
    }
    svg.push_str("</svg>");
    let texture = ui::svg_texture(ctx, "filmstrip", svg.as_bytes(), [936, 72]).unwrap();
    (
        icon,
        ui::Filmstrip::new(texture, 13, ui::StripAxis::Horizontal).unwrap(),
    )
}
impl eframe::App for Gallery {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.frames += 1;
        if let Some(loader) = &mut self.loader {
            if let Some(done) = loader.poll() {
                match done.result {
                    Ok(loaded) => {
                        self.thumbnail = Some(loaded.thumbnail);
                        self.cache_hit = loaded.cache_hit;
                    }
                    Err(error) => self.wave_error = Some(error.to_string()),
                }
            }
        }

        if let Some(path) = &self.snapshot {
            for event in root.ctx().input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .expect("save screenshot");
                    let report = serde_json::json!({"theme":themes()[self.theme],"native_gpu_screenshot":true,"dimensions":image.size,"svg_icon":true,"svg_filmstrip_frames":13,"thumbnail_frames":self.thumbnail.as_ref().map(|t|t.frames()),"thumbnail_cache_hit":self.cache_hit,"controls":["knobs","modulation ring","fader","toggle","enum","SVG button","filmstrip"]});
                    std::fs::write(
                        path.with_extension("json"),
                        serde_json::to_vec_pretty(&report).unwrap(),
                    )
                    .unwrap();
                    root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            if self.frames >= 12
                && !self.requested
                && (self.loader.is_none() || self.thumbnail.is_some() || self.wave_error.is_some())
            {
                root.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.requested = true;
            }
        }
        root.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
        if self.waveform_only {
            egui::CentralPanel::default().show(root, |ui| {
                ui.heading("Soniclaw / Waveform overview");
                if let Some(thumbnail) = &self.thumbnail {
                    ui.label(format!(
                        "{:.1} seconds · {} channels · {} Hz",
                        thumbnail.frames() as f64 / thumbnail.sample_rate() as f64,
                        thumbnail.channels(),
                        thumbnail.sample_rate()
                    ));
                    ui.add_space(18.);
                    for channel in 0..thumbnail.channels().min(4) {
                        ui.label(format!("Channel {}", channel + 1));
                        let _ = self.thumbnail_view.show(ui, thumbnail, channel);
                        ui.add_space(16.);
                    }
                    ui.label("Zoom and drag to explore the recording");
                } else if let Some(error) = &self.wave_error {
                    ui.label(error);
                } else {
                    ui.label("Loading waveform…");
                }
            });
            return;
        }
        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Soniclaw / Audio UI");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let previous = self.theme;
                        egui::ComboBox::from_id_salt("palette")
                            .selected_text(&themes()[self.theme].name)
                            .show_ui(ui, |ui| {
                                for (i, theme) in themes().iter().enumerate() {
                                    ui.selectable_value(&mut self.theme, i, &theme.name);
                                }
                            });
                        if previous != self.theme {
                            let theme = &themes()[self.theme];
                            theme.apply(ui.ctx()).unwrap();
                            (self.icon, self.strip) = assets(ui.ctx(), theme);
                        }
                    });
                });
                ui.label("Reusable controls for native audio applications");
                ui.add_space(12.);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label("FILTER & OUTPUT");
                    ui.columns(4, |columns| {
                        ui::audio_knob(
                            &mut columns[0],
                            self.cutoff.raw(),
                            ui::KnobOptions {
                                modulation: Some(self.modulation),
                                ..Default::default()
                            },
                        );
                        ui::audio_knob(&mut columns[1], self.gain.raw(), Default::default());
                        columns[1].add_space(12.);
                        ui::audio_toggle(&mut columns[1], self.enabled.raw());
                        ui::parameter_control(&mut columns[1], self.mode.raw());
                        ui::audio_fader(&mut columns[2], self.gain.raw(), 160.);
                        columns[3].label("Linked filmstrip");
                        self.strip
                            .show(&mut columns[3], self.gain.raw(), egui::Vec2::splat(72.));
                        if columns[3]
                            .add(egui::Button::image(egui::Image::new((
                                self.icon.id(),
                                egui::Vec2::splat(24.),
                            ))))
                            .on_hover_text("Toggle enabled")
                            .clicked()
                        {
                            self.enabled.set(!self.enabled.get()).unwrap();
                        }
                        columns[3].label("Output");
                        ui::level_meter(&mut columns[3], self.gain.get());
                    });
                    ui.add(
                        egui::Slider::new(&mut self.modulation, -0.5..=0.5).text("Modulation ring"),
                    );
                });
                ui.add_space(10.);
                ui.label("Tone response · drag to edit cutoff");
                self.frequency.set(self.cutoff.get()).unwrap();
                if ui::eq_editor(ui, &self.frequency, 0.707, 48000.).changed() {
                    self.cutoff.set(self.frequency.get()).unwrap();
                }
                ui.columns(2, |columns| {
                    columns[0].label("Envelope");
                    ui::envelope_editor(&mut columns[0], &mut self.adsr);
                    columns[1].label("XY modulation");
                    ui::xy_pad(&mut columns[1], self.cutoff.raw(), self.gain.raw());
                });
                ui.label("Waveform · zoom and drag");
                if let Some(thumbnail) = &self.thumbnail {
                    let _ = self.thumbnail_view.show(ui, thumbnail, 0);
                } else if let Some(error) = &self.wave_error {
                    ui.label(error);
                } else if let Some(loader) = &self.loader {
                    ui.label(format!("Loading waveform: {:?}", loader.progress()));
                } else {
                    self.wave.show(ui, &self.samples, 1);
                }
                ui.add_space(8.);
                for message in ui::piano_keyboard(ui, 60, 2, &mut self.held) {
                    self.last_note = format!("{message:?}");
                }
                ui.label(&self.last_note);
            });
        });
    }
}
fn main() -> Result<(), eframe::Error> {
    let args: Vec<_> = std::env::args().collect();
    let theme = args
        .iter()
        .position(|a| a == "--theme")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(2);
    let snapshot = args
        .iter()
        .position(|a| a == "--snapshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    if let Some(path) = &snapshot {
        assert!(!path.exists(), "snapshot path must be new");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
    }
    let waveform_only = args.iter().any(|a| a == "--waveform-only");
    let loader = if let Some(index) = args.iter().position(|a| a == "--audio") {
        let path = PathBuf::from(args.get(index + 1).expect("--audio requires path"));
        let cache = args
            .iter()
            .position(|a| a == "--cache")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("soniclaw-waveforms"));
        let mut loader = soniclaw::waveform::Loader::new(cache).expect("waveform worker");
        loader
            .request(path, Default::default())
            .expect("waveform request");
        Some(loader)
    } else {
        None
    };
    eframe::run_native(
        "Soniclaw Audio UI",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([920., 940.]),
            ..Default::default()
        },
        Box::new(move |cc| {
            themes()[theme].apply(&cc.egui_ctx).unwrap();
            let (icon, strip) = assets(&cc.egui_ctx, &themes()[theme]);
            Ok(Box::new(Gallery {
                cutoff: FloatParam::new(
                    Definition::new(
                        "cutoff",
                        "Cutoff",
                        Kind::Float {
                            min: 20.,
                            max: 20000.,
                            default: 3000.,
                            curve: Curve::Logarithmic,
                        },
                    )
                    .unit("Hz"),
                )
                .unwrap(),
                gain: FloatParam::new(Definition::new(
                    "gain",
                    "Gain",
                    Kind::Float {
                        min: 0.,
                        max: 1.,
                        default: 0.65,
                        curve: Curve::Linear,
                    },
                ))
                .unwrap(),
                enabled: BoolParam::new(Definition::new(
                    "enabled",
                    "Enabled",
                    Kind::Boolean { default: true },
                ))
                .unwrap(),
                mode: EnumParam::new("mode", "Character", Mode::Warm).unwrap(),
                frequency: Parameter::new(ParameterInfo {
                    id: "eq",
                    name: "Cutoff",
                    min: 20.,
                    max: 20000.,
                    default: 3000.,
                    unit: "Hz",
                })
                .unwrap(),
                adsr: Default::default(),
                wave: Default::default(),
                samples: (0..12000)
                    .map(|i| {
                        ((i as f32 * 0.09).sin() * 0.6 + (i as f32 * 0.021).sin() * 0.2)
                            * (1. - i as f32 / 12000.)
                    })
                    .collect(),
                held: [false; 128],
                last_note: String::new(),
                theme,
                modulation: 0.15,
                icon,
                strip,
                loader,
                thumbnail: None,
                thumbnail_view: Default::default(),
                cache_hit: false,
                wave_error: None,
                waveform_only,
                snapshot,
                frames: 0,
                requested: false,
            }))
        }),
    )
}
