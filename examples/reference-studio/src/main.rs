//! Shared multisample factory, hosted natively. No external samples required.
use soniclaw::{
    device::{event_channel, EventSender, OutputHost, Telemetry},
    prelude::*,
    ui::{self, egui},
};
use std::{sync::Arc, time::Duration};
struct Studio {
    host: OutputHost,
    sender: EventSender,
    gain: Arc<Parameter>,
    _midi: Option<soniclaw::midi::native::Connection>,
}
impl eframe::App for Studio {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        root.ctx().request_repaint_after(Duration::from_millis(33));
        egui::CentralPanel::default().show(root,|ui|{ui.heading("Soniclaw / Multisample Studio");ui.label("The same instrument factory runs standalone, in CLAP, and through the VST3 wrapper.");ui.add_space(16.0);ui::parameter_knob(ui,&self.gain,false);ui::level_meter(ui,self.host.telemetry.take_peak());ui.add_space(16.0);ui.horizontal(|ui|{for (name,note,key)in[("Kick / A",36,egui::Key::A),("Snare / S",38,egui::Key::S),("Hat / D",42,egui::Key::D)]{let pressed=ui.add_sized([140.0,90.0],egui::Button::new(name)).clicked()||ui.input(|i|i.key_pressed(key));if pressed{self.sender.send(MidiMessage::NoteOn{channel:0,note,velocity:110});}}});ui.add_space(10.0);ui.label("Two velocity layers × two round-robin variations per drum · 64 voices");ui.small("Starts muted. Raise Gain to hear it. Optional --midi <port-index>.");});
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|a| a == "--list-devices") {
        for (i, n) in OutputHost::devices()?.iter().enumerate() {
            println!("Audio {i}: {n}");
        }
        for (i, n) in soniclaw::midi::native::input_ports()?.iter().enumerate() {
            println!("MIDI {i}: {n}");
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--render") {
        let path = args
            .get(i + 1)
            .ok_or("--render requires an output WAV path")?;
        let mut e = Engine::default();
        e.push(soniclaw::plugin::HostedProcessor(
            soniclaw_reference::create_drums(),
        ));
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 256,
            channels: 2,
        })?;
        let events: Vec<_> = (0..16)
            .flat_map(|step| {
                let frame = step * 12000;
                let mut v = vec![Event {
                    frame,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 42,
                        velocity: if step % 2 == 0 { 105 } else { 65 },
                    },
                }];
                if step % 4 == 0 {
                    v.push(Event {
                        frame,
                        message: MidiMessage::NoteOn {
                            channel: 0,
                            note: 36,
                            velocity: 115,
                        },
                    });
                }
                if step % 4 == 2 {
                    v.push(Event {
                        frame,
                        message: MidiMessage::NoteOn {
                            channel: 0,
                            note: 38,
                            velocity: 110,
                        },
                    });
                }
                v
            })
            .collect();
        let data = soniclaw::engine::render(&mut e, 240000, &events)?;
        soniclaw::io::write_wav(path, 48000, 2, &data)?;
        println!("Rendered {path}");
        return Ok(());
    }
    let instance = soniclaw_reference::create_drums();
    let gain = instance.parameters()[0].clone();
    gain.set(0.0)?;
    let mut engine = Engine::default();
    engine.push(soniclaw::plugin::HostedProcessor(instance));
    let telemetry = Arc::new(Telemetry::default());
    let (sender, receiver) = event_channel(256, telemetry.clone());
    let (mut midi_sender, midi_receiver) = event_channel(256, telemetry.clone());
    let midi = if let Some(i) = args.iter().position(|a| a == "--midi") {
        let port = args.get(i + 1).ok_or("--midi requires an index")?.parse()?;
        Some(soniclaw::midi::native::connect_input(port, move |m| {
            midi_sender.send(m);
        })?)
    } else {
        None
    };
    let host = OutputHost::start(engine, None, 256, vec![receiver, midi_receiver], telemetry)?;
    let app = Studio {
        host,
        sender,
        gain,
        _midi: midi,
    };
    eframe::run_native(
        "Soniclaw Multisample Studio",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([620.0, 360.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            ui::apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}
