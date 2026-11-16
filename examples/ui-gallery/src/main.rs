//! Audio-independent component gallery for framework developers.
use soniclaw::{
    prelude::*,
    ui::{self, egui},
};
struct Gallery {
    frequency: Parameter,
    level: Parameter,
    adsr: AdsrSettings,
    wave: ui::WaveformView,
    samples: Vec<f32>,
    held: [bool; 128],
    last_note: String,
}
impl eframe::App for Gallery {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(root,|view|{egui::ScrollArea::vertical().show(view,|view|{
        view.heading("Soniclaw · Audio component gallery");view.label("Interactive UI components for application developers. This gallery does not open an audio device.");
        view.horizontal(|view|{ui::parameter_knob(view,&self.frequency,true);ui::parameter_knob(view,&self.level,false);ui::xy_pad(view,&self.frequency,&self.level);});
        view.label("Low-pass response · drag to set cutoff");ui::eq_editor(view,&self.frequency,0.707,48000.0);
        view.label("Envelope editor");ui::envelope_editor(view,&mut self.adsr);
        view.label("Waveform · zoom and drag to pan");self.wave.show(view,&self.samples,1);
        view.label("Piano keyboard · emitted MIDI messages");for message in ui::piano_keyboard(view,60,2,&mut self.held){self.last_note=format!("{message:?}");}view.label(&self.last_note);
    });});
    }
}
fn main() -> Result<(), eframe::Error> {
    let p = |id, name, min, max, default| {
        Parameter::new(ParameterInfo {
            id,
            name,
            min,
            max,
            default,
            unit: "",
        })
        .expect("valid gallery parameter")
    };
    let app = Gallery {
        frequency: p("cutoff", "Cutoff", 20.0, 20000.0, 3000.0),
        level: p("level", "Level", 0.0, 1.0, 0.5),
        adsr: Default::default(),
        wave: Default::default(),
        samples: (0..12000)
            .map(|i| {
                ((i as f32 * 0.09).sin() * 0.6 + (i as f32 * 0.021).sin() * 0.2)
                    * (1.0 - i as f32 / 12000.0)
            })
            .collect(),
        held: [false; 128],
        last_note: String::new(),
    };
    eframe::run_native(
        "Soniclaw component gallery",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 900.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            ui::apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
