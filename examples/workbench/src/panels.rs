use super::*;
use soniclaw::{dsp::voice_mod::*, midi::performance::*};
impl Workbench {
    fn publish_live(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.graph_control.is_some() {
            let p = self.model.build()?;
            let mut notes = vec![];
            if p.descriptor.is_instrument {
                for (i, note) in [60, 64, 67].into_iter().enumerate() {
                    notes.push(MidiMessage::NoteOnId {
                        channel: 1,
                        note,
                        velocity: 0.65,
                        note_id: i as i32 + 1,
                    });
                }
                for (note, held) in self.held.iter().enumerate() {
                    if *held {
                        notes.push(MidiMessage::NoteOn {
                            channel: 0,
                            note: note as u8,
                            velocity: 100,
                        });
                    }
                }
            }
            if let Err(e) = self
                .graph_control
                .as_mut()
                .expect("control")
                .replace_with_notes(
                    Box::new(soniclaw::project::ProjectProcessor::new(p)),
                    &notes,
                )
            {
                self.stop();
                return Err(e.into());
            }
            self.status = "Queued prepared graph · 2048-frame budget · new audition voices".into();
        }
        self.preview();
        Ok(())
    }
    pub(super) fn panel_action(&mut self, a: Action) -> Result<(), Box<dyn std::error::Error>> {
        match a {
            Action::GraphApply => {
                if let Some(d) = &self.graph_draft {
                    self.model.apply_routing(d)?;
                }
                self.publish_live()?;
            }
            Action::SamplesApply => {
                if let Some(d) = &self.sample_draft {
                    self.model.apply_samples(d)?;
                }
                self.publish_live()?;
            }
            Action::MidiApply => {
                let mut s = self.model.snapshot();
                s.midi = Some(self.midi_draft.clone());
                self.model.apply(s, "MIDI performance")?;
                self.publish_live()?;
            }
            Action::VoiceApply => {
                if let Some((id, cfg)) = &self.voice_draft {
                    let mut s = self.model.snapshot();
                    s.nodes
                        .iter_mut()
                        .find(|n| n.id == *id)
                        .ok_or("select voice source")?
                        .voice_modulation = Some(cfg.clone());
                    self.model.apply(s, "Per-voice modulation")?;
                    self.publish_live()?;
                }
            }
            Action::Audit => {
                if self.audit_worker.is_none() {
                    let s = self.model.snapshot();
                    let root = self.model.root.clone();
                    let (tx, rx) = mpsc::sync_channel(1);
                    std::thread::spawn(move || {
                        let r = soniclaw::diagnostics::audit(
                            || {
                                let p = soniclaw::project::create_plugin(
                                    soniclaw_workbench::app_descriptor(&s),
                                    s.clone(),
                                    &root,
                                )
                                .map_err(|_| Error::InvalidParameter)?;
                                Ok(Box::new(soniclaw::project::ProjectProcessor::new(p)))
                            },
                            &soniclaw::diagnostics::AuditConfig {
                                sample_rates: vec![44100.0, 48000.0],
                                block_sizes: vec![17, 64, 257],
                                frames: 4096,
                                channels: 2,
                            },
                        )
                        .map_err(|e| e.to_string());
                        let _ = tx.send(r);
                    });
                    self.audit_worker = Some(rx);
                    self.status = "Auditing snapshot on a worker…".into();
                }
            }
            Action::HostScan => {
                self.stop();
                let mut found = vec![];
                for path in soniclaw::host::discover(Path::new(&self.host_path))
                    .map_err(|e| e.to_string())?
                {
                    // SAFETY: this explicit action selects trusted native CLAP code.
                    let lib = unsafe { soniclaw::host::ClapLibrary::load(&path) }
                        .map_err(|e| e.to_string())?;
                    for p in lib.plugins() {
                        found.push((path.clone(), p.clone()));
                    }
                }
                self.host_products = found;
                self.host_selected = 0;
                self.host_parameters.clear();
                self.host_state = None;
                self.status = format!("Discovered {} products", self.host_products.len());
            }
            Action::HostInspect => {
                self.stop();
                let (path, info) = self
                    .host_products
                    .get(self.host_selected)
                    .ok_or("scan/select a product")?;
                // SAFETY: same explicitly selected trusted library as discovery.
                let lib = unsafe { soniclaw::host::ClapLibrary::load(path) }
                    .map_err(|e| e.to_string())?;
                let p = lib.instantiate(&info.id).map_err(|e| e.to_string())?;
                self.host_parameters = p.parameters().to_vec();
                self.host_state = None;
            }
            Action::HostRender => {
                self.stop();
                let (path, info) = self
                    .host_products
                    .get(self.host_selected)
                    .ok_or("scan/select a product")?;
                let state = self
                    .host_state
                    .as_ref()
                    .filter(|(id, _)| id == &info.id)
                    .map(|(_, s)| s.as_slice());
                // SAFETY: explicit trusted library; audio joins before teardown.
                let p = unsafe {
                    soniclaw_workbench::render_hosted(path, &info.id, &self.host_parameters, state)
                }
                .map_err(|e| e.to_string())?;
                let channels = p.audio.len() / 96000;
                let file = self.model.root.join("hosted-preview.wav");
                soniclaw::io::write_wav(&file, 48000, channels as u16, &p.audio)?;
                self.host_state = Some((info.id.clone(), p.state));
                self.host_parameters = p.parameters;
                self.wave = p
                    .audio
                    .chunks_exact(channels)
                    .step_by(96)
                    .map(|f| f[0])
                    .collect();
                self.peak = p.audio.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
                self.latency = p.latency;
                self.rate = 48000.0;
                self.cpu = (p.processing_ms / 2000.0 * 100.0) as f32;
                let mut mono = [0.0; 1024];
                for (i, x) in mono.iter_mut().enumerate() {
                    *x = p.audio.get((24000 + i) * channels).copied().unwrap_or(0.0);
                }
                if let Ok(b) = self.analyzer.analyze(&mono) {
                    self.spectrum = b.to_vec();
                }
                self.status = format!("Hosted render and state round trip: {}", file.display());
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn poll_panels(&mut self) {
        if let Some(rx) = &self.audit_worker {
            match rx.try_recv() {
                Ok(r) => {
                    self.audit_worker = None;
                    match r {
                        Ok(r) => {
                            self.status = format!(
                                "Audit: {} configurations, finite={}",
                                r.cases.len(),
                                r.finite
                            );
                            self.audit_report = Some(r);
                        }
                        Err(e) => self.status = e,
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.audit_worker = None;
                    self.status = "Audit worker disconnected".into();
                }
                Err(_) => {}
            }
        }
    }
    pub(super) fn graph_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("Routing graph");
        if self.graph_draft.is_none() {
            self.graph_draft = Some(self.model.snapshot());
        }
        egui::ScrollArea::vertical()
            .id_salt("routing-panel")
            .max_height(440.0)
            .show(ui, |ui| {
                self.graph_editor
                    .show(ui, self.graph_draft.as_mut().unwrap());
            });
        ui.horizontal(|ui| {
            if ui.button("Apply graph").clicked() {
                *a = Some(Action::GraphApply);
            }
            if ui.button("Discard draft").clicked() {
                self.graph_draft = None;
            }
        });
        ui.label("Warm new graph, crossfade, retire on control. No processor-history transfer.");
    }
    pub(super) fn samples_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("Sampler key / velocity map");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.sfz_path);
            if ui.button("Import SFZ subset").clicked() {
                self.stop();
                match self.loader.submit(
                    soniclaw::project_loading::LoadRequest::Sfz {
                        path: self.sfz_path.clone().into(),
                        product: soniclaw_workbench::APP_ID.into(),
                    },
                    None,
                ) {
                    Ok(_) => self.loading = true,
                    Err(e) => self.status = e.into(),
                }
            }
        });

        if self.sample_draft.is_none() {
            self.sample_draft = Some(self.model.snapshot());
        }
        let d = self.sample_draft.as_mut().unwrap();
        if let Some(asset) = d.assets.get(
            self.sample_editor
                .asset
                .min(d.assets.len().saturating_sub(1)),
        ) {
            let key = format!("{}:{}", self.model.root.display(), asset.path);
            if self.sample_cache != key {
                self.sample_cache = key;
                self.sample_wave.clear();
                self.sample_frames = 0;
                match d
                    .resolve_asset(&self.model.root, asset)
                    .and_then(|p| soniclaw::io::read_wav_with_limit(p, 128 * 1024 * 1024))
                {
                    Ok(w) => {
                        self.sample_frames = w.samples.len() / w.channels as usize;
                        let step = (self.sample_frames / 1500).max(1);
                        self.sample_wave = w
                            .samples
                            .chunks_exact(w.channels as usize)
                            .step_by(step)
                            .map(|f| f.iter().sum::<f32>() / w.channels as f32)
                            .collect();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        if ui.button("Detect transients in selected asset").clicked()
            && self.transient_worker.is_none()
        {
            if let Some(asset) = d.assets.get(self.sample_editor.asset).cloned() {
                let root = self.model.root.clone();
                let state = d.clone();
                let key = format!("{}:{}", asset.id, asset.path);
                let (tx, rx) = mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
                        let data = soniclaw::io::read_wav_with_limit(
                            state.resolve_asset(&root, &asset)?,
                            128 * 1024 * 1024,
                        )?;
                        let markers = soniclaw::sampling::detect_transients(
                            &data,
                            soniclaw::sampling::TransientOptions {
                                max_slices: 92,
                                ..Default::default()
                            },
                        )?;
                        Ok((key, markers, data.frames()))
                    })()
                    .map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });
                self.transient_worker = Some(rx);
            }
        }
        if let Some(rx) = &self.transient_worker {
            if let Ok(result) = rx.try_recv() {
                self.transient_worker = None;
                match result {
                    Ok(v) => self.transient_ready = Some(v),
                    Err(e) => self.status = e,
                }
            }
        }
        if let Some((key, markers, frames)) = &self.transient_ready {
            ui.label(format!("Detected {} slices", markers.len()));
            if ui
                .button("Replace selected asset zones with detected slices (keys 36+)")
                .clicked()
            {
                if let Some(asset) = d.assets.get(self.sample_editor.asset) {
                    if *key == format!("{}:{}", asset.id, asset.path) {
                        match soniclaw::sampling::sliced_zones(&asset.id, markers, *frames, 36) {
                            Ok(zones) => {
                                let id = asset.id.clone();
                                d.zones.retain(|z| z.asset != id);
                                d.zones.extend(zones);
                                self.transient_ready = None;
                            }
                            Err(e) => self.status = e.to_string(),
                        }
                    } else {
                        self.status = "Selected asset changed; detect again".into();
                    }
                }
            }
        }
        egui::ScrollArea::vertical()
            .id_salt("samples-panel")
            .max_height(440.0)
            .show(ui, |ui| {
                self.sample_editor
                    .show(ui, d, &self.sample_wave, self.sample_frames);
            });
        ui.horizontal(|ui| {
            if ui.button("Apply sample map").clicked() {
                *a = Some(Action::SamplesApply);
            }
            if ui.button("Discard sample draft").clicked() {
                self.sample_draft = None;
                self.sample_cache.clear();
            }
        });
        ui.label("Sampler Studio uses the memory sampler for regions/slices/crossfades.");
    }
    pub(super) fn midi_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("MIDI performance");
        let c = &mut self.midi_draft;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("arp-mode")
                .selected_text(format!("{:?}", c.mode))
                .show_ui(ui, |ui| {
                    for mode in [
                        ArpMode::Off,
                        ArpMode::Up,
                        ArpMode::Down,
                        ArpMode::UpDown,
                        ArpMode::Random,
                    ] {
                        ui.selectable_value(&mut c.mode, mode, format!("{mode:?}"));
                    }
                });
            for (label, notes) in [
                ("Single", vec![0]),
                ("Major", vec![0, 4, 7]),
                ("Minor", vec![0, 3, 7]),
                ("Maj7", vec![0, 4, 7, 11]),
            ] {
                if ui.button(label).clicked() {
                    c.chord = notes;
                }
            }
        });
        ui.label(format!("Chord semitones: {:?}", c.chord));
        ui.horizontal(|ui| {
            ui.label("Scale root");
            ui.add(egui::DragValue::new(&mut c.scale_root).range(0..=11));
            for (label, mask) in [
                ("Chromatic", 0xfff),
                ("Major scale", 0b101010110101),
                ("Minor scale", 0b010110101101),
            ] {
                ui.selectable_value(&mut c.scale_mask, mask, label);
            }
        });
        ui.add(egui::Slider::new(&mut c.step_beats, 0.0625..=4.0).text("Beats / step"));
        ui.add(egui::Slider::new(&mut c.gate, 0.05..=0.95).text("Gate"));
        ui.add(egui::Slider::new(&mut c.swing, 0.0..=0.49).text("Swing"));
        ui.add(egui::Slider::new(&mut c.humanize_ms, 0.0..=50.0).text("Timing jitter ms"));
        ui.add(egui::Slider::new(&mut c.velocity_jitter, 0..=64).text("Velocity jitter"));
        if ui.button("Apply MIDI performance").clicked() {
            *a = Some(Action::MidiApply);
        }
    }
    pub(super) fn voice_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("Per-voice modulation");
        let Some(n) = self
            .model
            .history
            .current()
            .nodes
            .iter()
            .find(|n| n.kind == ProcessorKind::PolyModSynth)
        else {
            ui.label("Load Poly Motion to edit independent note envelopes/LFOs.");
            return;
        };
        if self.voice_draft.as_ref().is_none_or(|(id, _)| id != &n.id) {
            self.voice_draft = Some((n.id.clone(), n.voice_modulation.clone().unwrap_or_default()));
        }
        let (_, cfg) = self.voice_draft.as_mut().unwrap();
        egui::ScrollArea::vertical()
            .id_salt("voice-config")
            .max_height(390.0)
            .show(ui, |ui| {
                ui.columns(2, |cols| {
                    for (i, col) in cols.iter_mut().enumerate() {
                        col.label(format!("Envelope {}", i + 1));
                        soniclaw::ui::envelope_editor(col, &mut cfg.envelopes[i]);
                        col.label(format!("LFO {}", i + 1));
                        col.add(
                            egui::Slider::new(&mut cfg.lfos[i].rate_hz, 0.01..=40.0)
                                .logarithmic(true)
                                .text("Hz"),
                        );
                        egui::ComboBox::from_id_salt(("voice-lfo", i))
                            .selected_text(format!("{:?}", cfg.lfos[i].shape))
                            .show_ui(col, |ui| {
                                for shape in [
                                    LfoShape::Sine,
                                    LfoShape::Triangle,
                                    LfoShape::Saw,
                                    LfoShape::Square,
                                    LfoShape::SampleHold,
                                ] {
                                    ui.selectable_value(
                                        &mut cfg.lfos[i].shape,
                                        shape,
                                        format!("{shape:?}"),
                                    );
                                }
                            });
                    }
                });
                let mut remove = None;
                for (i, r) in cfg.routes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("voice-source", i))
                            .selected_text(format!("{:?}", r.source))
                            .show_ui(ui, |ui| {
                                for s in [
                                    VoiceSource::Envelope1,
                                    VoiceSource::Envelope2,
                                    VoiceSource::Lfo1,
                                    VoiceSource::Lfo2,
                                    VoiceSource::Velocity,
                                    VoiceSource::Key,
                                    VoiceSource::Pressure,
                                    VoiceSource::Timbre,
                                ] {
                                    ui.selectable_value(&mut r.source, s, format!("{s:?}"));
                                }
                            });
                        egui::ComboBox::from_id_salt(("voice-dest", i))
                            .selected_text(format!("{:?}", r.destination))
                            .show_ui(ui, |ui| {
                                for d in [
                                    VoiceDestination::Pitch,
                                    VoiceDestination::Cutoff,
                                    VoiceDestination::Level,
                                    VoiceDestination::Pan,
                                ] {
                                    ui.selectable_value(&mut r.destination, d, format!("{d:?}"));
                                }
                            });
                        ui.add(egui::Slider::new(&mut r.amount, -1.0..=1.0).text("Depth"));
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    cfg.routes.remove(i);
                }
                if cfg.routes.len() < 32 && ui.button("Add voice route").clicked() {
                    cfg.routes.push(VoiceRoute {
                        source: VoiceSource::Lfo1,
                        destination: VoiceDestination::Pitch,
                        amount: 0.01,
                    });
                }
            });
        if ui.button("Apply voice modulation").clicked() {
            *a = Some(Action::VoiceApply);
        }
    }
    pub(super) fn diagnostics_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("Developer diagnostics");
        let s = self.live_stats.snapshot();
        ui.label(format!(
            "Live blocks {} · max {:.3} ms · over-budget {} · non-finite {}",
            s.blocks,
            s.max_ns as f64 / 1e6,
            s.overruns,
            s.nonfinite
        ));
        ui.label("Timing depends on clocks/OS scheduling; these are not driver xrun counts.");
        if ui
            .add_enabled(
                self.audit_worker.is_none(),
                egui::Button::new("Audit rack snapshot"),
            )
            .clicked()
        {
            *a = Some(Action::Audit);
        }
        if let Some(r) = &self.audit_report {
            ui.label(format!(
                "{} configurations · finite={} · heap probe {:?}",
                r.cases.len(),
                r.finite,
                r.allocation_clean
            ));
            egui::ScrollArea::vertical().id_salt("audit-rows").max_height(270.0).show(ui,|ui|{for c in &r.cases{ui.label(format!("{:.0} Hz / {} frames: {} blocks, peak {:.3}, latency {}, worst {:.2}× budget",c.rate,c.max_block,c.stats.blocks,c.peak,c.latency,c.max_block_ratio));}});
            if ui.button("Save report").clicked() {
                self.status = match serde_json::to_string_pretty(r)
                    .map_err(|e| e.to_string())
                    .and_then(|s| {
                        std::fs::write(self.model.root.join("diagnostics.json"), s)
                            .map_err(|e| e.to_string())
                    }) {
                    Ok(()) => "Saved diagnostics.json".into(),
                    Err(e) => e,
                };
            }
        }
        ui.label(
            "Heap probes require the diagnostics allocations feature in a development executable.",
        );
    }
    pub(super) fn host_panel(&mut self, ui: &mut egui::Ui, a: &mut Option<Action>) {
        ui.heading("CLAP host");
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.host_direct,
                "Direct low-latency mode (trusted DSP)",
            );
            if !self.host_direct {
                ui.label("Lookahead packets");
                ui.add(egui::DragValue::new(&mut self.host_buffer_blocks).range(1..=7));
            }
        });
        ui.label("Loads native code in this process. Select trusted plug-ins. Floating editors supported when advertised; no sidechain.");
        ui.text_edit_singleline(&mut self.host_path);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.host_session.is_none(),
                    egui::Button::new("Add selected to rack"),
                )
                .clicked()
            {
                if let Some((path, info)) = self.host_products.get(self.host_selected) {
                    if self.host_chain.len() < soniclaw::host::live::MAX_NODES {
                        self.host_chain.push(soniclaw::host::live::RackEntry {
                            path: path.clone(),
                            plugin_id: info.id.clone(),
                        });
                    }
                }
            }
            if ui.button("Start live rack").clicked() {
                self.vst3_requested = false;
                self.host_requested = true;
                if let Err(e) = self.play() {
                    self.status = e;
                }
            }
            if ui.button("Stop rack").clicked() {
                self.stop();
            }
            if ui.button("Clear rack").clicked() {
                self.stop();
                self.host_chain.clear();
                self.host_session = None;
                self.host_missing.clear();
            }
        });
        for (i, entry) in self.host_chain.iter().enumerate() {
            if ui
                .selectable_value(
                    &mut self.host_node,
                    i,
                    format!("{}. {}", i + 1, entry.plugin_id),
                )
                .changed()
            {
                if let Some(rack) = &self.host_live {
                    self.host_parameters = self
                        .host_mapping
                        .iter()
                        .position(|n| *n == i)
                        .and_then(|n| rack.parameters(n))
                        .unwrap_or(&[])
                        .to_vec();
                }
            }
        }
        if let Some(rack) = &mut self.host_live {
            ui.label(format!("{:?}", rack.stats().snapshot()));
            ui.horizontal(|ui| {
                ui.label("Editor scale");
                ui.add(
                    egui::DragValue::new(&mut self.host_editor_scale)
                        .range(0.5..=4.0)
                        .speed(0.1),
                );
                if ui.button("Apply scale").clicked() {
                    let node = self
                        .host_mapping
                        .iter()
                        .position(|i| *i == self.host_node)
                        .unwrap_or(usize::MAX);
                    if let Err(e) = rack.scale_editor(node, self.host_editor_scale) {
                        self.status = e.to_string();
                    }
                }
                ui.add(egui::DragValue::new(&mut self.host_editor_size[0]).range(64..=8192));
                ui.add(egui::DragValue::new(&mut self.host_editor_size[1]).range(64..=8192));
                if ui.button("Resize editor").clicked() {
                    let node = self
                        .host_mapping
                        .iter()
                        .position(|i| *i == self.host_node)
                        .unwrap_or(usize::MAX);
                    if let Err(e) = rack.resize_editor(node, self.host_editor_size) {
                        self.status = e.to_string();
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Show embedded editor").clicked() {
                    let node = self
                        .host_mapping
                        .iter()
                        .position(|i| *i == self.host_node)
                        .unwrap_or(usize::MAX);
                    if let Err(e) = rack.show_embedded_editor(node, "Soniclaw hosted editor") {
                        self.status = e.to_string();
                    }
                }
                if ui.button("Show floating editor").clicked() {
                    if let Err(e) = rack.show_editor(
                        self.host_mapping
                            .iter()
                            .position(|i| *i == self.host_node)
                            .unwrap_or(usize::MAX),
                    ) {
                        self.status = e.to_string();
                    }
                }
                if ui.button("Close editor").clicked() {
                    rack.close_editor(
                        self.host_mapping
                            .iter()
                            .position(|i| *i == self.host_node)
                            .unwrap_or(usize::MAX),
                    );
                }
            });
        }
        for missing in &self.host_missing {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Missing slot {}: {}", missing.slot + 1, missing.reason),
            );
        }
        for warning in &self.host_warnings {
            ui.label(warning);
        }
        if let Some(rack) = &mut self.host_live {
            if let Some(node) = self.host_mapping.iter().position(|i| *i == self.host_node) {
                let mut bypass = rack.is_bypassed(node);
                if ui
                    .checkbox(&mut bypass, "Bypass selected (DSP continues)")
                    .changed()
                {
                    if let Err(e) = rack.set_bypass(node, bypass) {
                        self.status = e.to_string();
                    }
                }
            }
        }
        ui.horizontal(|ui| {
            let path = self.model.root.join("host-session.json");
            if ui.button("Save full session (stops audio)").clicked() {
                self.save_host_session(&path);
            }
            if ui.button("Load full session").clicked() {
                self.stop();
                match soniclaw::host::session::Session::read(&path) {
                    Ok(session) => {
                        self.host_chain = session.slots.iter().map(|s| s.entry.clone()).collect();
                        self.host_session = Some(session);
                        self.host_node = 0;
                        self.status =
                            "Session loaded; Start explicitly restores trusted native code".into();
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        });
        ui.horizontal(|ui| {
            let path = self.model.root.join("host-rack.json");
            if ui.button("Save rack list").clicked() {
                match serde_json::to_vec_pretty(&self.host_chain)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                {
                    Ok(()) => self.status = format!("Saved {}", path.display()),
                    Err(e) => self.status = e,
                }
            }
            if ui.button("Load rack list").clicked() {
                self.stop();
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        if bytes.len() > 65536 {
                            return Err("rack list exceeds 64 KiB".into());
                        }
                        serde_json::from_slice::<Vec<soniclaw::host::live::RackEntry>>(&bytes)
                            .map_err(|e| e.to_string())
                    }) {
                    Ok(entries) if entries.len() <= 8 => {
                        self.host_chain = entries;
                        self.host_session = None;
                        self.host_missing.clear();
                        self.host_node = 0;
                    }
                    Ok(_) => self.status = "rack exceeds 8 nodes".into(),
                    Err(e) => self.status = e,
                }
            }
        });

        ui.horizontal(|ui| {
            if ui.button("Discover / scan").clicked() {
                *a = Some(Action::HostScan);
            }
            if ui.button("Inspect product").clicked() {
                *a = Some(Action::HostInspect);
            }
            if ui.button("Render hosted + state round trip").clicked() {
                *a = Some(Action::HostRender);
            }
        });
        for (i, (_, p)) in self.host_products.iter().enumerate() {
            if ui
                .selectable_value(&mut self.host_selected, i, format!("{} — {}", p.name, p.id))
                .changed()
            {
                self.host_parameters.clear();
                self.host_state = None;
            }
        }
        egui::ScrollArea::vertical()
            .id_salt("host-controls")
            .max_height(280.0)
            .show(ui, |ui| {
                for p in &mut self.host_parameters {
                    if ui
                        .add_enabled(
                            p.flags & 8 == 0,
                            egui::Slider::new(&mut p.value, p.min..=p.max).text(&p.name),
                        )
                        .changed()
                    {
                        if let Some(rack) = &mut self.host_live {
                            if let Err(e) = rack.set_parameter(
                                self.host_mapping
                                    .iter()
                                    .position(|i| *i == self.host_node)
                                    .unwrap_or(usize::MAX),
                                p.id,
                                p.value,
                            ) {
                                self.status = e.to_string();
                            }
                        }
                    }
                }
            });
        ui.label("Scoped preview joins audio before main-thread state restore. Writes hosted-preview.wav; opens no audio device.");
    }
}

impl Workbench {
    pub(super) fn automation_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Automation lanes");
        ui.label("Record gestures into a draft while audio runs. Apply lanes to hear the edited timeline; Save persists the applied document.");
        if self.automation_draft.is_none() {
            self.automation_draft = Some(self.model.snapshot().automation.unwrap_or_default());
        }
        let ids: Vec<_> = self
            .model
            .parameters
            .iter()
            .map(|p| p.info().id.to_owned())
            .collect();
        if let Err(e) =
            self.automation_editor
                .show(ui, self.automation_draft.as_mut().expect("draft"), &ids)
        {
            ui.colored_label(egui::Color32::RED, format!("Invalid draft: {e}"));
        }
        ui.horizontal(|ui| {
            if ui.button("Apply automation").clicked() {
                let mut state = self.model.snapshot();
                state.automation = self.automation_draft.clone();
                match self
                    .model
                    .apply(state, "Edit automation")
                    .and_then(|_| self.publish_live())
                {
                    Ok(()) => self.status = "Automation applied".into(),
                    Err(e) => self.status = e.to_string(),
                }
            }
            if ui.button("Reload saved lanes").clicked() {
                self.automation_draft = Some(self.model.snapshot().automation.unwrap_or_default());
            }
        });
    }
}

impl Workbench {
    fn save_host_session(&mut self, path: &Path) {
        let Some(mut owner) = self.host_live.take() else {
            if let Some(session) = &self.host_session {
                self.status = match session.write(path) {
                    Ok(()) => "Saved hosted session".into(),
                    Err(e) => e.to_string(),
                };
            } else {
                self.status = "Start the rack before capturing its native state".into();
            }
            return;
        };
        let entries: Vec<_> = self
            .host_mapping
            .iter()
            .filter_map(|i| self.host_chain.get(*i).cloned())
            .collect();
        self.stop();
        let result = soniclaw::host::session::Session::capture(
            &mut owner,
            &entries,
            "Workbench hosted rack",
        )
        .and_then(|capture| {
            let mut session = self.host_session.clone().unwrap_or_else(|| capture.clone());
            if self.host_session.is_some() {
                session.merge_capture(capture, &self.host_mapping)?;
            }
            session.write(path)?;
            self.host_session = Some(session);
            Ok(())
        });
        self.status = match result {
            Ok(()) => "Full native session saved; Start restores it".into(),
            Err(e) => e.to_string(),
        };
    }
}

impl Workbench {
    pub(super) fn transport_panel(&mut self, ui: &mut egui::Ui) {
        use soniclaw::transport::*;
        ui.heading("Musical transport and tempo map");
        ui.label("Quarter-note beats / PPQ. Apply publishes the map to the internal project; host rack timing is a separate API.");
        let timeline = self
            .timeline_draft
            .get_or_insert_with(|| self.model.snapshot().timeline.unwrap_or_default());
        ui.horizontal(|ui| {
            if ui.button("Add tempo point").clicked() && timeline.tempo.len() < 1024 {
                let last = timeline.tempo.last().copied().expect("default tempo");
                timeline.tempo.push(TempoPoint {
                    beat: last.beat + 4.0,
                    bpm: last.bpm,
                    transition: Transition::Step,
                });
            }
            ui.checkbox(&mut timeline.panic_on_loop, "Release notes on loop");
        });
        let mut remove = None;
        for (i, p) in timeline.tempo.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Tempo {i}"));
                ui.add_enabled(
                    i > 0,
                    egui::DragValue::new(&mut p.beat).range(0.0..=1_000_000.0),
                );
                ui.add(egui::DragValue::new(&mut p.bpm).range(20.0..=400.0));
                ui.selectable_value(&mut p.transition, Transition::Step, "Step");
                ui.selectable_value(&mut p.transition, Transition::Linear, "Ramp");
                if i > 0 && ui.button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            timeline.tempo.remove(i);
        }
        ui.label("Meter changes must fall on a previous-meter bar boundary.");
        if ui.button("Add meter at next bar").clicked() && timeline.meters.len() < 1024 {
            let last = timeline.meters.last().copied().expect("default meter");
            let length = last.numerator as f64 * 4.0 / last.denominator as f64;
            timeline.meters.push(MeterPoint {
                beat: last.beat + length,
                numerator: 4,
                denominator: 4,
            });
        }
        let mut remove = None;
        for (i, p) in timeline.meters.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("Meter");
                ui.add_enabled(
                    i > 0,
                    egui::DragValue::new(&mut p.beat).range(0.0..=1_000_000.0),
                );
                ui.add(egui::DragValue::new(&mut p.numerator).range(1..=32));
                ui.add(egui::DragValue::new(&mut p.denominator).range(1..=64));
                if i > 0 && ui.button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            timeline.meters.remove(i);
        }
        for (label, range) in [
            ("Loop", &mut timeline.loop_range),
            ("Punch", &mut timeline.punch_range),
        ] {
            ui.horizontal(|ui| {
                let mut enabled = range.is_some();
                if ui.checkbox(&mut enabled, label).changed() {
                    *range = enabled.then_some([0.0, 4.0]);
                }
                if let Some([a, b]) = range {
                    ui.add(egui::DragValue::new(a).range(0.0..=1_000_000.0));
                    ui.add(egui::DragValue::new(b).range(0.0..=1_000_000.0));
                }
            });
        }
        if let Err(e) = TempoMap::compile(timeline) {
            ui.colored_label(egui::Color32::RED, format!("Invalid timeline: {e}"));
        }
        if ui.button("Apply musical timeline").clicked() {
            let mut state = self.model.snapshot();
            state.timeline = self.timeline_draft.clone();
            match self
                .model
                .apply(state, "Edit musical timeline")
                .and_then(|_| self.publish_live())
            {
                Ok(()) => {
                    self.status = "Musical timeline applied; its map controls project tempo".into()
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if ui.button("Reload timeline").clicked() {
            self.timeline_draft = Some(self.model.snapshot().timeline.unwrap_or_default());
        }
        if ui.button("Disable project tempo map").clicked() {
            let mut state = self.model.snapshot();
            state.timeline = None;
            match self
                .model
                .apply(state, "Disable musical timeline")
                .and_then(|_| self.publish_live())
            {
                Ok(()) => self.status = "Project uses ordinary host/device transport again".into(),
                Err(e) => self.status = e.to_string(),
            }
        }
    }
}

impl Workbench {
    pub(super) fn vst3_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("VST3 host");
        ui.checkbox(
            &mut self.vst3_direct,
            "Direct audio mode (stop audio before opening an editor; MIDI uses buffered mode)",
        );
        ui.add_enabled_ui(!self.vst3_direct, |ui| {
            ui.checkbox(
                &mut self.vst3_musical,
                "Use project tempo map, meter and loop range",
            );
        });
        ui.label("Select a trusted native bundle. Stereo hosting uses a 1024-frame worker bridge.");
        ui.text_edit_singleline(&mut self.vst3_path);
        if ui.button("Inspect trusted bundle").clicked() {
            self.stop();
            self.vst3_state = None;
            // SAFETY: this explicit action selects trusted native code for factory inspection.
            match unsafe {
                soniclaw::host::formats::products(
                    soniclaw::host::formats::PluginFormat::Vst3,
                    Path::new(&self.vst3_path),
                )
            } {
                Ok(p) => {
                    self.vst3_products = p;
                    self.vst3_selected = 0;
                    self.status = "VST3 factory inspected".into();
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        let mut changed = false;
        for (i, p) in self.vst3_products.iter().enumerate() {
            changed |= ui
                .selectable_value(&mut self.vst3_selected, i, &p.name)
                .changed();
        }
        if changed {
            self.stop();
            self.vst3_state = None;
        }
        ui.horizontal(|ui| {
            if ui.button("Start VST3 audio").clicked() {
                self.host_requested = false;
                self.vst3_requested = true;
                if let Err(e) = self.play() {
                    self.status = e;
                }
            }
            if ui.button("Stop VST3 audio").clicked() {
                self.stop();
            }
            if ui.button("Open editor / controls").clicked() {
                let result = (|| -> Result<(), String> {
                    if self.vst3_live.is_none() {
                        let p = self
                            .vst3_products
                            .get(self.vst3_selected)
                            .ok_or("select a VST3 product")?;
                        // SAFETY: explicit trusted product selection.
                        let mut owner = unsafe {
                            soniclaw::vst3::Instance::load(
                                &p.path,
                                Some(&p.id),
                                ProcessSpec {
                                    sample_rate: self.rate,
                                    max_block_frames: 256,
                                    channels: 2,
                                },
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        if let Some(state) = &self.vst3_state {
                            owner.restore_state(state).map_err(|e| e.to_string())?;
                        }
                        self.vst3_parameters = owner.parameters().map_err(|e| e.to_string())?;
                        self.vst3_live = Some(owner);
                    }
                    self.vst3_live
                        .as_mut()
                        .unwrap()
                        .open_editor()
                        .map_err(|e| e.to_string())
                })();
                if let Err(e) = result {
                    self.status = e;
                }
            }
            if ui.button("Close editor").clicked() {
                if let Some(p) = &mut self.vst3_live {
                    p.close_editor();
                }
            }
        });
        ui.text_edit_singleline(&mut self.vst3_state_path);
        ui.horizontal(|ui| {
            if ui.button("Save stopped VST3 state").clicked() {
                self.stop();
                let result = (|| -> Result<(), String> {
                    let s = self
                        .vst3_state
                        .as_ref()
                        .ok_or("open/start a product first")?;
                    s.write_atomic(Path::new(&self.vst3_state_path))
                        .map_err(|e| e.to_string())
                })();
                self.status = result
                    .map(|_| "VST3 state saved".into())
                    .unwrap_or_else(|e| e);
            }
            if ui.button("Load VST3 state").clicked() {
                self.stop();
                let result = soniclaw::vst3::State::read(Path::new(&self.vst3_state_path))
                    .map_err(|e| e.to_string());
                match result {
                    Ok(s) => {
                        self.vst3_state = Some(s);
                        self.status = "State validated; start matching product to restore".into();
                    }
                    Err(e) => self.status = e,
                }
            }
        });
        if let Some(owner) = &mut self.vst3_live {
            ui.label(format!("{:?}", owner.stats()));
            for p in &mut self.vst3_parameters {
                let mut value = p.value;
                if ui
                    .add_enabled(
                        !p.is_read_only,
                        egui::Slider::new(&mut value, 0.0..=1.0).text(&p.name),
                    )
                    .changed()
                {
                    match owner.set_parameter(p.id, value) {
                        Ok(()) => p.value = value,
                        Err(e) => self.status = e.to_string(),
                    }
                }
            }
        }
    }
}
