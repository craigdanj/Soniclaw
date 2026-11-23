//! Editable arrangement, JSON persistence and continuous offline preview rendering.
use soniclaw::{
    clips::*,
    prelude::*,
    ui::{self, egui},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
fn demo() -> (Document, Assets) {
    let rate = 48000;
    let asset = AudioAsset {
        sample_rate: rate,
        channels: 1,
        samples: Arc::from(
            (0..rate)
                .map(|i| (i as f32 * 110. * std::f32::consts::TAU / rate as f32).sin() * 0.08)
                .collect::<Vec<_>>(),
        ),
    };
    let doc = Document {
        schema: 1,
        sample_rate: rate,
        channels: 2,
        timeline: Default::default(),
        audio: vec![AudioClip {
            edits: None,
            id: "Bass-loop".into(),
            asset: "bass".into(),
            track: 0,
            start_frame: 0,
            length_frames: rate as u64 * 4,
            source_offset: 0,
            source_length: rate as u64,
            looping: true,
            gain: 1.,
            fade_in: 2400,
            fade_out: 9600,
            enabled: true,
        }],
        midi: vec![MidiClip {
            controls: vec![],
            id: "Melody".into(),
            track: 1,
            start_beat: 0.,
            length_beats: 8.,
            source_offset: 0.,
            source_length: 4.,
            looping: true,
            notes: [60, 64, 67, 71]
                .into_iter()
                .enumerate()
                .map(|(i, key)| Note {
                    id: i as u32,
                    beat: i as f64,
                    length: 0.75,
                    channel: 0,
                    key,
                    velocity: 0.65,
                })
                .collect(),
            enabled: true,
        }],
    };
    (doc, Assets::from([("bass".into(), asset)]))
}
fn render(
    doc: &Document,
    assets: &Assets,
    block: usize,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let instrument = Box::new(ExpressiveSynth::<16>::default());
    let mut player = doc.compile(assets, block, Some(instrument))?;
    let mut out = vec![0.; 48000 * 5 * 2];
    for (n, chunk) in out.chunks_mut(block * 2).enumerate() {
        player.render(
            &mut AudioBuffer::new(chunk, 2)?,
            Transport {
                sample_position: (n * block) as u64,
                ..Default::default()
            },
        )?;
    }
    Ok(out)
}
fn export(doc: &Document, assets: &Assets, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let pcm = render(doc, assets, 256)?;
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: doc.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for s in pcm {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}
struct App {
    history: soniclaw::commands::History<Document>,
    commands: soniclaw::commands::Registry,
    doc: Document,
    assets: Assets,
    clips: ui::ClipEditor,
    roll: ui::PianoRollEditor,
    status: String,
    snapshot: Option<PathBuf>,
    frames: u32,
    requested: bool,
    out: PathBuf,
}
impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.frames += 1;
        if let Some(path) = &self.snapshot {
            for e in root.ctx().input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = e {
                    let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                    root.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            if self.frames > 10 && !self.requested {
                root.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.requested = true;
            }
        }
        self.commands
            .state("edit.undo", self.history.can_undo(), false)
            .unwrap();
        self.commands
            .state("edit.redo", self.history.can_redo(), false)
            .unwrap();
        let mut action = ui::command_shortcuts(root.ctx(), &self.commands);
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("Soniclaw / Clip studio");
            ui.horizontal(|ui| {
                ui.menu_button("Edit", |ui| {
                    if let Some(id) = ui::command_menu(ui, &self.commands, "Edit") {
                        action = Some(id)
                    }
                });
                for id in ["edit.undo", "edit.redo"] {
                    if ui::command_button(ui, &self.commands, id) {
                        action = Some(id.into())
                    }
                }
            });
            if let Some(id) = action {
                match id.as_str() {
                    "edit.undo" => {
                        self.history.undo(&mut self.doc);
                    }
                    "edit.redo" => {
                        self.history.redo(&mut self.doc);
                    }
                    _ => {}
                }
            }
            let before = self.doc.clone();
            let mut changed = false;

            ui.label("Drag regions and notes · double click the piano roll to add a note");
            ui.horizontal(|ui| {
                if ui.button("Save arrangement").clicked() {
                    self.status = match self.doc.to_json().and_then(|text| {
                        std::fs::write(self.out.join("arrangement.json"), text)
                            .map_err(|_| soniclaw::core::Error::InvalidParameter)
                    }) {
                        Ok(()) => "Arrangement saved".into(),
                        Err(e) => e.to_string(),
                    };
                }
                if ui.button("Render preview WAV").clicked() {
                    self.status =
                        match export(&self.doc, &self.assets, &self.out.join("preview.wav")) {
                            Ok(()) => "Five-second preview rendered".into(),
                            Err(e) => e.to_string(),
                        };
                }
            });
            match self.clips.show(ui, &mut self.doc) {
                Ok(value) => changed |= value,
                Err(e) => self.status = e.to_string(),
            }
            ui.separator();
            ui.heading("Melody / source piano roll");
            if self.doc.midi.iter().any(|c| c.id == "Melody") {
                match self.roll.show(ui, &mut self.doc, "Melody") {
                    Ok(value) => changed |= value,
                    Err(e) => self.status = e.to_string(),
                }
            }
            if changed {
                self.history.record(before, "Clip edit");
            }
            ui.label(&self.status);
        });
        root.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(
        args.get(1)
            .ok_or("usage: soniclaw-clip-studio NEW_DIRECTORY [--gui] [--snapshot FILE]")?,
    );
    std::fs::create_dir(&out)?;
    let (doc, assets) = demo();
    let a = render(&doc, &assets, 64)?;
    let b = render(&doc, &assets, 256)?;
    let error = a
        .iter()
        .zip(&b)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(error < 1e-6);
    assert!(a.iter().any(|v| v.abs() > 0.01));
    export(&doc, &assets, &out.join("preview.wav"))?;
    std::fs::write(out.join("arrangement.json"), doc.to_json()?)?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"partition_error":error,"frames":a.len()/2,"peak":a.iter().map(|v|v.abs()).fold(0f32,f32::max)}),
        )?,
    )?;
    if args.iter().any(|a| a == "--gui") {
        let snapshot = args
            .iter()
            .position(|a| a == "--snapshot")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from);
        eframe::run_native(
            "Soniclaw Clip studio",
            eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default().with_inner_size([960., 800.]),
                ..Default::default()
            },
            Box::new(move |cc| {
                ui::Theme::dark().apply(&cc.egui_ctx).unwrap();
                Ok(Box::new(App {
                    history: soniclaw::commands::History::new(128).unwrap(),
                    commands: command_registry(),
                    doc,
                    assets,
                    clips: ui::ClipEditor {
                        selected: Some("Bass-loop".into()),
                        snap_frames: 6000,
                        snap_beats: 0.25,
                        ..Default::default()
                    },
                    roll: ui::PianoRollEditor {
                        snap_beats: 0.25,
                        ..Default::default()
                    },
                    status: "Preview ready; edits apply to the draft. Render to hear changes."
                        .into(),
                    snapshot,
                    frames: 0,
                    requested: false,
                    out,
                }))
            }),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn command_registry() -> soniclaw::commands::Registry {
    use soniclaw::commands::*;
    let mut registry = Registry::default();
    let mut map = Keymap::default();
    for (id, label, key) in [("edit.undo", "Undo", "Z"), ("edit.redo", "Redo", "Y")] {
        registry
            .register(Command {
                id: id.into(),
                label: label.into(),
                category: "Edit".into(),
                enabled: true,
                checked: false,
            })
            .unwrap();
        let mut shortcut = Shortcut::new(key);
        shortcut.control = true;
        map.bindings.insert(id.into(), shortcut);
    }
    registry.set_keymap(map).unwrap();
    registry
}
