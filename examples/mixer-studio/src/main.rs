use soniclaw::{
    mixer::*,
    ui::{self, egui},
};
use std::path::{Path, PathBuf};
fn render(mixer: &mut Mixer, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut main = hound::WavWriter::create(path.join("mix.wav"), spec)?;
    let mut send = hound::WavWriter::create(path.join("send.wav"), spec)?;
    for block in 0..375 {
        let a: Vec<_> = (0..256)
            .flat_map(|i| {
                let s =
                    ((block * 256 + i) as f32 * 220. * std::f32::consts::TAU / 48000.).sin() * 0.2;
                [s, s]
            })
            .collect();
        let b: Vec<_> = (0..256)
            .flat_map(|i| {
                let s =
                    ((block * 256 + i) as f32 * 330. * std::f32::consts::TAU / 48000.).sin() * 0.2;
                [s, s]
            })
            .collect();
        let mut out = [0.; 512];
        let mut aux = [0.; 512];
        mixer.process(&[&a, &b], &mut out, &mut [&mut aux], &[])?;
        for v in out {
            main.write_sample(v)?;
        }
        for v in aux {
            send.write_sample(v)?;
        }
    }
    main.finalize()?;
    send.finalize()?;
    std::fs::write(path.join("mixer.json"), mixer.snapshot().to_json()?)?;
    Ok(())
}
struct App {
    mixer: Mixer,
    path: PathBuf,
    status: String,
}
impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("Soniclaw / Mixer strips");
            ui::mixer_controls(ui, &self.mixer);
            if ui.button("Render two-second mix and send stems").clicked() {
                self.status = match render(&mut self.mixer, &self.path) {
                    Ok(()) => "Rendered mix.wav and send.wav".into(),
                    Err(e) => e.to_string(),
                };
            }
            ui.label(&self.status);
        });
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = PathBuf::from(
        args.get(1)
            .ok_or("usage: soniclaw-mixer-studio NEW_DIRECTORY [--gui]")?,
    );
    std::fs::create_dir(&path)?;
    let model = Model {
        schema: 1,
        strips: [("left", -0.5), ("right", 0.5)]
            .into_iter()
            .map(|(id, pan)| Strip {
                id: id.into(),
                name: id.into(),
                gain: 0.8,
                pan,
                mute: false,
                solo: false,
                group: Some(0),
                sends: vec![0.25],
            })
            .collect(),
        groups: vec![Group {
            gain: 1.,
            mute: false,
        }],
        taps: vec![Tap::PostFader],
    };
    let mut mixer = model.compile(48000., 256)?;
    render(&mut mixer, &path)?;
    let peaks: Vec<_> = mixer.controls().iter().map(|c| c.take_peaks()).collect();
    assert!(peaks.iter().flatten().all(|v| *v > 0.));
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":96000,"post_fader_strip_peaks":peaks}),
        )?,
    )?;
    if args.iter().any(|a| a == "--gui") {
        eframe::run_native(
            "Soniclaw Mixer",
            eframe::NativeOptions::default(),
            Box::new(move |cc| {
                ui::Theme::dark().apply(&cc.egui_ctx).unwrap();
                Ok(Box::new(App {
                    mixer,
                    path,
                    status: "Set controls then render to export the new mix.".into(),
                }))
            }),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
