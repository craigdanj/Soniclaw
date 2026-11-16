use soniclaw::{
    device::{event_channel, EventSender, OutputHost, Telemetry},
    prelude::*,
    ui::egui,
};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

struct Studio {
    host: OutputHost,
    sender: EventSender,
    gain: Arc<Parameter>,
    cutoff: Arc<Parameter>,
    held: [bool; 13],
    peak: f32,
    _midi: Option<soniclaw::midi::native::Connection>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|a| a == "--list-devices") {
        for (index, name) in OutputHost::devices()?.iter().enumerate() {
            println!("Audio {index}: {name}");
        }
        for (index, name) in soniclaw::midi::native::input_ports()?.iter().enumerate() {
            println!("MIDI {index}: {name}");
        }
        return Ok(());
    }
    let index = |flag: &str| -> Result<Option<usize>, Box<dyn std::error::Error>> {
        match args.iter().position(|a| a == flag) {
            Some(i) => Ok(Some(
                args.get(i + 1).ok_or("missing device index")?.parse()?,
            )),
            None => Ok(None),
        }
    };
    let gain = Arc::new(Parameter::new(ParameterInfo {
        id: "master.gain",
        name: "Master",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        unit: "",
    })?);
    let cutoff = Arc::new(Parameter::new(ParameterInfo {
        id: "filter.cutoff",
        name: "Cutoff",
        min: 20.0,
        max: 20000.0,
        default: 3000.0,
        unit: " Hz",
    })?);
    let mut engine = Engine::default();
    engine.push(PolySynth::<16>::default());
    engine.push(LowPass::new(cutoff.clone()));
    engine.push(Delay::new(0.3, 0.3, 0.22)?);
    engine.push(Gain::new(gain.clone()));
    let telemetry = Arc::new(Telemetry::default());
    let (sender, receiver) = event_channel(256, telemetry.clone());
    let (mut midi_sender, midi_receiver) = event_channel(256, telemetry.clone());
    let midi = if let Some(port) = index("--midi")? {
        Some(soniclaw::midi::native::connect_input(
            port,
            move |message| {
                midi_sender.send(message);
            },
        )?)
    } else {
        None
    };
    let host = OutputHost::start(
        engine,
        index("--device")?,
        256,
        vec![receiver, midi_receiver],
        telemetry,
    )?;
    let studio = Studio {
        host,
        sender,
        gain,
        cutoff,
        held: [false; 13],
        peak: 0.0,
        _midi: midi,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([780.0, 510.0])
            .with_min_inner_size([620.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Soniclaw · First Light",
        options,
        Box::new(|cc| {
            soniclaw::ui::apply_theme(&cc.egui_ctx);
            Ok(Box::new(studio))
        }),
    )?;
    Ok(())
}

impl eframe::App for Studio {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(16));
        self.peak = self.host.telemetry.take_peak().max(self.peak * 0.90);
        let mut desired = [false; 13];
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let keys = [
            egui::Key::A,
            egui::Key::W,
            egui::Key::S,
            egui::Key::E,
            egui::Key::D,
            egui::Key::F,
            egui::Key::T,
            egui::Key::G,
            egui::Key::Y,
            egui::Key::H,
            egui::Key::U,
            egui::Key::J,
            egui::Key::K,
        ];
        egui::CentralPanel::default().show(root, |ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("S O N I C L A W  /  F I R S T  L I G H T")
                    .color(egui::Color32::from_rgb(67, 216, 172)),
            );
            ui.add_space(8.0);
            ui.heading(egui::RichText::new("A small instrument. A bigger foundation.").size(27.0));
            ui.label("16 voices  ·  PolyBLEP saw  ·  ADSR  ·  Low-pass  ·  Stereo delay");
            ui.add_space(20.0);
            ui.group(|ui| {
                ui.set_min_width(ui.available_width());
                soniclaw::ui::parameter_slider(ui, &self.gain, false);
                soniclaw::ui::parameter_slider(ui, &self.cutoff, true);
                ui.add_space(8.0);
                soniclaw::ui::level_meter(ui, self.peak);
            });
            ui.add_space(16.0);
            ui.label("Hold the keys below, or play A W S E D F T G Y H U J K.");
            ui.horizontal(|ui| {
                for (i, name) in [
                    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B", "C",
                ]
                .iter()
                .enumerate()
                {
                    let black = name.contains('♯');
                    let fill = if self.held[i] {
                        egui::Color32::from_rgb(28, 130, 109)
                    } else if black {
                        egui::Color32::from_rgb(37, 45, 59)
                    } else {
                        egui::Color32::from_rgb(217, 222, 226)
                    };
                    let color = if black || self.held[i] {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_rgb(20, 25, 35)
                    };
                    let response = ui.add_sized(
                        [35.0, if black { 75.0 } else { 105.0 }],
                        egui::Button::new(egui::RichText::new(*name).color(color))
                            .fill(fill)
                            .sense(egui::Sense::click_and_drag()),
                    );
                    desired[i] = focused
                        && (response.is_pointer_button_down_on()
                            || (!ctx.egui_wants_keyboard_input()
                                && ctx.input(|input| input.key_down(keys[i]))));
                }
            });
            ui.add_space(14.0);
            if ui.button("All notes off").clicked() {
                self.sender.all_notes_off();
                desired.fill(false);
            }
            ui.separator();
            ui.small(format!(
                "{}  ·  {} Hz  ·  {} channels  ·  Internal block ≤ {} frames",
                self.host.device_name,
                self.host.spec.sample_rate,
                self.host.spec.channels,
                self.host.spec.max_block_frames
            ));
            let t = &self.host.telemetry;
            ui.small(format!(
                "Stream errors: {}  /  Processing errors: {}  /  Dropped events: {}",
                t.stream_errors.load(Ordering::Relaxed),
                t.process_errors.load(Ordering::Relaxed),
                t.dropped_events.load(Ordering::Relaxed)
            ));
        });
        for (i, &down) in desired.iter().enumerate() {
            if down != self.held[i] {
                let message = if down {
                    MidiMessage::NoteOn {
                        channel: 0,
                        note: 60 + i as u8,
                        velocity: 100,
                    }
                } else {
                    MidiMessage::NoteOff {
                        channel: 0,
                        note: 60 + i as u8,
                        velocity: 0,
                    }
                };
                self.sender.send(message);
            }
        }
        self.held = desired;
    }
}
