use soniclaw::{
    device::DeviceCatalog,
    prelude::*,
    record::RecordSource,
    session::{native::SessionController, SessionSettings},
    ui::{self, egui},
};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
struct LiveFx {
    session: SessionController,
    catalog: DeviceCatalog,
    gain: Arc<Parameter>,
    cutoff: Arc<Parameter>,
    input_meter: ui::MeterBallistics,
    output_meter: ui::MeterBallistics,
    waveform: ui::WaveformView,
    recorded: Vec<f32>,
    recorded_channels: usize,
    record_path: String,
    message: String,
    dry: bool,
}
fn parameter(
    id: &'static str,
    name: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: &'static str,
) -> Arc<Parameter> {
    Arc::new(
        Parameter::new(ParameterInfo {
            id,
            name,
            min,
            max,
            default,
            unit,
        })
        .expect("valid built-in parameter"),
    )
}
fn settings_path_for_load() -> PathBuf {
    PathBuf::from("soniclaw-session.json")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gain = parameter("master.gain", "Output", 0.0, 1.0, 0.5, "");
    let cutoff = parameter("filter.cutoff", "Cutoff", 20.0, 20000.0, 8000.0, " Hz");
    let g = gain.clone();
    let c = cutoff.clone();
    let settings_path = settings_path_for_load();
    let (mut settings, mut message) = (SessionSettings::default(), String::new());
    if settings_path.exists() {
        match SessionSettings::load(&settings_path) {
            Ok(saved) => settings = saved,
            Err(error) => message = format!("Could not load saved settings: {error}"),
        }
    }
    let session = SessionController::new(settings, move || {
        let mut engine = Engine::default();
        engine.push(LowPass::new(c.clone()));
        engine.push(Delay::new(0.25, 0.25, 0.15).expect("valid built-in delay"));
        engine.push(Gain::new(g.clone()));
        engine
    });
    let catalog = DeviceCatalog::enumerate().unwrap_or_default();
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let app = LiveFx {
        session,
        catalog,
        gain,
        cutoff,
        input_meter: Default::default(),
        output_meter: Default::default(),
        waveform: Default::default(),
        recorded: vec![],
        recorded_channels: 2,
        record_path: format!("soniclaw-recording-{timestamp}.wav"),
        message,
        dry: false,
    };
    eframe::run_native(
        "Soniclaw · Live FX & Recorder",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1020.0, 800.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            ui::apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}
impl LiveFx {
    fn result(&mut self, result: Result<(), Box<dyn std::error::Error>>) {
        self.message = match result {
            Ok(()) => "Done".into(),
            Err(error) => error.to_string(),
        };
    }
    fn stop_recording(&mut self) {
        let result = self.session.stop_recording();
        if let Err(error) = result {
            self.message = error.to_string();
            return;
        }
        self.message = format!("Saved {}", self.record_path);
        if std::fs::metadata(&self.record_path).is_ok_and(|m| m.len() <= 20 * 1024 * 1024) {
            if let Ok(file) = soniclaw::io::read_wav(&self.record_path) {
                self.recorded = file.samples;
                self.recorded_channels = file.channels as usize;
            }
        }
    }
}
impl eframe::App for LiveFx {
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(30));
        self.session.poll();
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 1.0);
        if let Some(active) = &self.session.active {
            self.input_meter
                .update(active.host.telemetry.take_input_peak(), dt);
            self.output_meter
                .update(active.host.telemetry.take_peak(), dt);
        } else {
            self.input_meter.update(0.0, dt);
            self.output_meter.update(0.0, dt);
        }
        egui::CentralPanel::default().show(root,|ui_ctx|{egui::ScrollArea::vertical().show(ui_ctx,|ui_ctx|{
            ui_ctx.label(egui::RichText::new("S O N I C L A W  /  L I V E  F X").color(egui::Color32::from_rgb(67,216,172)));ui_ctx.heading("Plug in. Shape the sound. Capture a take.");
            ui_ctx.label("Low-pass → delay → output gain. Monitoring starts muted; use headphones for live microphone input.");ui_ctx.add_space(12.0);
            ui_ctx.collapsing("Audio devices and session",|ui_ctx|{
                egui::Grid::new("devices").num_columns(2).spacing([20.0,10.0]).show(ui_ctx,|ui_ctx|{
                    ui_ctx.label("Input");egui::ComboBox::from_id_salt("input").selected_text(self.session.settings.input_device.as_deref().unwrap_or("Disabled")).show_ui(ui_ctx,|ui_ctx|{ui_ctx.selectable_value(&mut self.session.settings.input_device,None,"Disabled");for device in &self.catalog.inputs{ui_ctx.selectable_value(&mut self.session.settings.input_device,Some(device.name.clone()),&device.name);}});ui_ctx.end_row();
                    ui_ctx.label("Output");egui::ComboBox::from_id_salt("output").selected_text(self.session.settings.output_device.as_deref().unwrap_or("System default")).show_ui(ui_ctx,|ui_ctx|{ui_ctx.selectable_value(&mut self.session.settings.output_device,None,"System default");for device in &self.catalog.outputs{ui_ctx.selectable_value(&mut self.session.settings.output_device,Some(device.name.clone()),&device.name);}});ui_ctx.end_row();
                    ui_ctx.label("Sample rate");egui::ComboBox::from_id_salt("rate").selected_text(format!("{} Hz",self.session.settings.sample_rate)).show_ui(ui_ctx,|ui_ctx|{for rate in[44100,48000,88200,96000,192000]{ui_ctx.selectable_value(&mut self.session.settings.sample_rate,rate,format!("{rate} Hz"));}});ui_ctx.end_row();
                    ui_ctx.label("Device buffer");egui::ComboBox::from_id_salt("buffer").selected_text(self.session.settings.buffer_frames.map_or("Driver default".into(),|n|format!("{n} frames"))).show_ui(ui_ctx,|ui_ctx|{ui_ctx.selectable_value(&mut self.session.settings.buffer_frames,None,"Driver default");for frames in[32,64,128,256,512,1024]{ui_ctx.selectable_value(&mut self.session.settings.buffer_frames,Some(frames),format!("{frames} frames"));}});ui_ctx.end_row();
                    ui_ctx.label("Input L / R channels (zero-based)");ui_ctx.horizontal(|ui_ctx|{ui_ctx.add(egui::DragValue::new(&mut self.session.settings.input_channels[0]).range(0..=63));ui_ctx.add(egui::DragValue::new(&mut self.session.settings.input_channels[1]).range(0..=63));});ui_ctx.end_row();
                    ui_ctx.label("Output L / R channels (zero-based)");ui_ctx.horizontal(|ui_ctx|{ui_ctx.add(egui::DragValue::new(&mut self.session.settings.output_channels[0]).range(0..=63));ui_ctx.add(egui::DragValue::new(&mut self.session.settings.output_channels[1]).range(0..=63));});ui_ctx.end_row();
                });
                ui_ctx.checkbox(&mut self.session.settings.auto_reconnect,"Reconnect after device loss");
                ui_ctx.horizontal(|ui_ctx|{
                    if ui_ctx.button("Refresh devices").clicked(){match DeviceCatalog::enumerate(){Ok(catalog)=>self.catalog=catalog,Err(error)=>self.message=error.to_string()}}
                    if ui_ctx.button("Save settings").clicked(){let result=self.session.settings.save("soniclaw-session.json");self.result(result);}
                    if ui_ctx.button("Load settings").clicked(){match SessionSettings::load(settings_path_for_load()){Ok(settings)=>{self.session.settings=settings;self.message="Settings loaded; apply to restart audio".into();},Err(error)=>self.message=error.to_string()}}
                });
                if let Some(device)=self.catalog.outputs.iter().find(|d|Some(&d.name)==self.session.settings.output_device.as_ref()){for config in device.configs.iter().take(6){ui_ctx.small(format!("{} ch · {}–{} Hz · {} · buffer {:?}",config.channels,config.min_rate,config.max_rate,config.format,config.buffer_range));}}
            });
            ui_ctx.horizontal(|ui_ctx|{
                if ui_ctx.button("Apply & start / restart").clicked(){let result=self.session.start();self.result(result);}
                if ui_ctx.button("Stop audio").clicked(){self.session.stop();}
                let mut monitor=self.session.controls.monitor.load(Ordering::Relaxed);if ui_ctx.checkbox(&mut monitor,"Monitor output").changed(){self.session.controls.monitor.store(monitor,Ordering::Relaxed);}
                let mut bypass=self.session.controls.bypass.load(Ordering::Relaxed);if ui_ctx.checkbox(&mut bypass,"Bypass effects").changed(){self.session.controls.bypass.store(bypass,Ordering::Relaxed);}
            });ui_ctx.label(&self.session.status);ui_ctx.separator();
            ui_ctx.horizontal(|ui_ctx|{
                ui::parameter_knob(ui_ctx,&self.cutoff,true);ui::parameter_knob(ui_ctx,&self.gain,false);
                ui_ctx.vertical(|ui_ctx|{ui_ctx.label("XY · cutoff / level");ui::xy_pad(ui_ctx,&self.cutoff,&self.gain);});
                ui_ctx.vertical(|ui_ctx|{ui_ctx.label("Input");self.input_meter.show(ui_ctx);ui_ctx.label("Processed output (before monitor mute)");self.output_meter.show(ui_ctx);});
            });
            ui::eq_editor(ui_ctx,&self.cutoff,std::f32::consts::FRAC_1_SQRT_2,self.session.active.as_ref().map_or(self.session.settings.sample_rate as f32, |s| s.host.spec.sample_rate as f32));ui_ctx.separator();
            let recording=self.session.active.as_ref().is_some_and(|a|a.recorder.is_recording());
            ui_ctx.horizontal(|ui_ctx|{
                ui_ctx.label("WAV destination");ui_ctx.add_enabled(!recording,egui::TextEdit::singleline(&mut self.record_path).desired_width(340.0));ui_ctx.add_enabled(!recording,egui::Checkbox::new(&mut self.dry,"Record dry input"));
                if !recording{if ui_ctx.button("Record").clicked(){let source=if self.dry{RecordSource::Dry}else{RecordSource::Processed};let result=self.session.record(&self.record_path,source);self.result(result);}}
                else if ui_ctx.button("Stop & finalize take").clicked(){self.stop_recording();}
            });
            if let Some(active)=&self.session.active{let stats=active.recorder.stats();ui_ctx.small(format!("Captured: {} frames · Written: {} · Recording loss: {} frames · Disk errors: {}",stats.captured_frames.load(Ordering::Relaxed),stats.written_frames.load(Ordering::Relaxed),stats.dropped_frames.load(Ordering::Relaxed),stats.write_errors.load(Ordering::Relaxed)));
                let t=&active.host.telemetry;ui_ctx.small(format!("Input underruns: {} · Input overflows: {} · Stream errors: {} · Software prefill: {} frames",t.input_underruns.load(Ordering::Relaxed),t.input_overflows.load(Ordering::Relaxed),t.stream_errors.load(Ordering::Relaxed),active.host.bridge_prefill_frames));}
            if !self.recorded.is_empty(){ui_ctx.label("Last finalized take");self.waveform.show(ui_ctx,&self.recorded,self.recorded_channels);}
            if let Some(summary)=&self.session.last_recording_summary{ui_ctx.small(format!("Last finalized take: {} frames · Lost {} frames · Disk errors {}",summary.written_frames,summary.dropped_frames,summary.write_errors));}
            if let Some(error)=&self.session.last_recording_error{ui_ctx.colored_label(egui::Color32::LIGHT_RED,error);}
            if !self.message.is_empty(){ui_ctx.label(&self.message);}
        });});
    }
}
