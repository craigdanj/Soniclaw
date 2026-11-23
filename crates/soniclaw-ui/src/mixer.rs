//! Reusable stereo strip controls. Processor ownership remains with the application.
pub fn mixer_controls(ui: &mut egui::Ui, mixer: &soniclaw_mixer::Mixer) {
    for (index, group) in mixer.groups().iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("Group {}", index + 1));
            let mut gain = group.gain();
            if ui
                .add(egui::Slider::new(&mut gain, 0.0..=4.0).text("Gain"))
                .changed()
            {
                let _ = group.set_gain(gain);
            }
            let mut mute = group.muted();
            if ui.checkbox(&mut mute, "Mute").changed() {
                group.set_mute(mute);
            }
        });
    }
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for (strip, controls) in mixer.model().strips.iter().zip(mixer.controls()) {
                ui.vertical(|ui| {
                    ui.set_min_width(140.);
                    ui.heading(&strip.name);
                    let mut gain = controls.gain();
                    if ui
                        .add(
                            egui::Slider::new(&mut gain, 0.0..=4.0)
                                .text(format!("{} gain", strip.name)),
                        )
                        .changed()
                    {
                        let _ = controls.set_gain(gain);
                    }
                    let mut pan = controls.pan();
                    if ui
                        .add(
                            egui::Slider::new(&mut pan, -1.0..=1.0)
                                .text(format!("{} pan", strip.name)),
                        )
                        .changed()
                    {
                        let _ = controls.set_pan(pan);
                    }
                    let mut mute = controls.muted();
                    if ui
                        .checkbox(&mut mute, format!("{} mute", strip.name))
                        .changed()
                    {
                        controls.set_mute(mute);
                    }
                    let mut solo = controls.soloed();
                    if ui
                        .checkbox(&mut solo, format!("{} solo", strip.name))
                        .changed()
                    {
                        controls.set_solo(solo);
                    }
                    for index in 0..strip.sends.len() {
                        let mut send = controls.send(index).unwrap();
                        if ui
                            .add(egui::Slider::new(&mut send, 0.0..=4.0).text(format!(
                                "{} send {}",
                                strip.name,
                                index + 1
                            )))
                            .changed()
                        {
                            let _ = controls.set_send(index, send);
                        }
                    }
                    let peaks = controls.take_peaks();
                    ui.label(format!("Peak L {:.3} / R {:.3}", peaks[0], peaks[1]));
                });
            }
        });
    });
}
