#[derive(Default)]
struct Tools {
    directory: String,
    query: String,
    entries: Vec<media_tools::Entry>,
    selected: usize,
    synth: usize,
    effect: usize,
    root: u8,
    file_tempo: bool,
    preset_name: String,
    preset_directory: String,
    processor: String,
    job: Option<jobs::Ticket<ToolsReady>>,
    baseline: Option<Project>,
    preview_done: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
enum ToolsReady {
    Entries(Vec<media_tools::Entry>),
    Audio {
        asset: soniclaw::clips::AudioAsset,
        track: String,
        beat: f64,
        sampler: Option<u8>,
    },
    Midi(Box<Project>),
    Preset(media_tools::Preset, String),
    Saved,
    Preview(soniclaw::clips::AudioAsset),
}
impl Studio {
    fn tools_action(&mut self, action: &str) -> Result<()> {
        if self.tools.job.is_some() {
            return Err("wait for the current media job".into());
        }
        let track = self.project.document.tracks[self.selected].id.clone();
        match action {
            "tools-synth" => {
                self.stop()?;
                stock::set_instrument(
                    &mut self.project,
                    &track,
                    stock::instrument(self.tools.synth, &workspace::identity("synth")?)?,
                )?;
            }
            "tools-effect" => {
                self.stop()?;
                stock::add_effect(
                    &mut self.project,
                    &track,
                    stock::effect(
                        stock::EFFECTS[self.tools.effect].0,
                        &workspace::identity("fx")?,
                    )?,
                )?;
            }
            "tools-insert-up" | "tools-insert-down" | "tools-insert-remove" => {
                self.stop()?;
                let direction = match action {
                    "tools-insert-up" => -1,
                    "tools-insert-down" => 1,
                    _ => 0,
                };
                stock::edit_insert(&mut self.project, &track, &self.tools.processor, direction)?;
            }
            "tools-scan" => {
                let dir = PathBuf::from(&self.tools.directory);
                let query = self.tools.query.clone();
                self.tools.job = Some(self.jobs.submit(
                    "tools",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    2 * 1024 * 1024,
                    move |_| {
                        media_tools::scan(&dir, &query)
                            .map(ToolsReady::Entries)
                            .map_err(|e| e.to_string())
                    },
                )?);
            }
            "tools-parent" => {
                self.tools.directory = PathBuf::from(&self.tools.directory)
                    .parent()
                    .ok_or("no parent directory")?
                    .to_string_lossy()
                    .into_owned();
                self.tools_action("tools-scan")?;
            }
            "tools-open" => {
                let e = self
                    .tools
                    .entries
                    .get(self.tools.selected)
                    .ok_or("select a folder")?;
                if e.kind != media_tools::Kind::Folder {
                    return Err("select an ordinary directory".into());
                }
                self.tools.directory = e.path.to_string_lossy().into_owned();
                self.tools_action("tools-scan")?;
            }
            "tools-audio" | "tools-sampler" | "tools-preview" | "tools-midi"
            | "tools-preset-load" => {
                self.stop()?;
                let e = self
                    .tools
                    .entries
                    .get(self.tools.selected)
                    .ok_or("select a media entry")?
                    .clone();
                let rate = self.project.document.sample_rate;
                let beat = self.play_from_beat;
                let sampler = if action == "tools-sampler" {
                    Some(self.tools.root)
                } else {
                    None
                };
                let preview = action == "tools-preview";
                let midi = action == "tools-midi";
                let preset = action == "tools-preset-load";
                let mut project = self.project.clone();
                let tempo = self.tools.file_tempo;
                if !preview {
                    self.tools.baseline = Some(project.clone());
                }
                self.tools.job = Some(self.jobs.submit(
                    "tools",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    192 * 1024 * 1024,
                    move |ctx| {
                        if ctx.cancelled() {
                            return Err("media job cancelled".into());
                        }
                        (|| -> Result<ToolsReady> {
                            if preset {
                                if e.kind != media_tools::Kind::Preset {
                                    return Err("select a stock preset folder".into());
                                }
                                return Ok(ToolsReady::Preset(
                                    media_tools::load_preset(&e.path, rate)?,
                                    track,
                                ));
                            }
                            if midi {
                                if e.kind != media_tools::Kind::Midi {
                                    return Err("select a MIDI file".into());
                                }
                                media_tools::import_midi(
                                    &mut project,
                                    &e.path,
                                    &track,
                                    beat,
                                    tempo,
                                )?;
                                return Ok(ToolsReady::Midi(Box::new(project)));
                            }
                            if e.kind != media_tools::Kind::Audio {
                                return Err("select an audio file".into());
                            }
                            let asset = media_tools::decode(&e.path, rate)?;
                            Ok(if preview {
                                ToolsReady::Preview(asset)
                            } else {
                                ToolsReady::Audio {
                                    asset,
                                    track,
                                    beat,
                                    sampler,
                                }
                            })
                        })()
                        .map_err(|e| e.to_string())
                    },
                )?);
            }
            "tools-preset-save" => {
                self.stop()?;
                let project = self.project.clone();
                let id = self.tools.processor.clone();
                let name = self.tools.preset_name.clone();
                let dir = PathBuf::from(&self.tools.preset_directory);
                self.tools.job = Some(self.jobs.submit(
                    "tools",
                    self.job_epoch,
                    jobs::Priority::Normal,
                    192 * 1024 * 1024,
                    move |_| {
                        media_tools::save_preset(&project, &id, &name, &dir)
                            .map(|_| ToolsReady::Saved)
                            .map_err(|e| e.to_string())
                    },
                )?);
            }
            _ => return Err("unknown media action".into()),
        }
        Ok(())
    }
    fn poll_tools(&mut self) {
        if self
            .tools
            .preview_done
            .as_ref()
            .is_some_and(|d| d.load(std::sync::atomic::Ordering::Acquire))
        {
            self.manager.stop();
            self.tools.preview_done = None;
        }
        let Some(result) = self.tools.job.as_mut().and_then(|t| t.poll(self.job_epoch)) else {
            return;
        };
        self.tools.job = None;
        let baseline = self.tools.baseline.take();
        let applied = (|| -> Result<()> {
            let ready = result?;
            if let Some(base) = baseline {
                if self.control.is_some()
                    || self.recording
                    || !history::equivalent(&base, &self.project)?
                {
                    return Err("project changed during import; select the file and retry".into());
                }
            }
            let previous = self.project.clone();
            match ready {
                ToolsReady::Entries(entries) => {
                    self.tools.entries = entries;
                    self.tools.selected = 0;
                }
                ToolsReady::Audio {
                    asset,
                    track,
                    beat,
                    sampler,
                } => {
                    let id = workspace::identity("sample")?;
                    if let Some(root) = sampler {
                        stock::install_sample(&mut self.project, &track, &id, asset, root)?;
                    } else {
                        media_tools::place_audio(
                            &mut self.project,
                            &track,
                            &id,
                            &workspace::identity("audio")?,
                            asset,
                            beat,
                        )?;
                    }
                }
                ToolsReady::Midi(p) => self.project = *p,
                ToolsReady::Preset(p, t) => media_tools::install_preset(&mut self.project, &t, p)?,
                ToolsReady::Saved => {}
                ToolsReady::Preview(asset) => {
                    self.stop()?;
                    let mut prefs = self.manager.preferences().clone();
                    let output = prefs.duplex.as_ref().map_or([0, 1], |d| d.output_channels);
                    prefs.duplex = Some(soniclaw::device::DuplexRoute {
cue_outputs:Vec::new(),

                        input_count: None,
                        input: None,
                        use_default_input: false,
                        input_channels: [0, 0],
                        output_channels: output,
                    });
                    self.manager = DeviceManager::new(NativeOutputBackend, prefs)
                        .map_err(|e| e.to_string())?;
                    let preview = media_tools::Preview::new(asset);
                    self.tools.preview_done = Some(preview.done.clone());
                    let mut preview = Some(preview);
                    self.manager
                        .start(self.started.elapsed(), &mut |_, _| {
                            let mut engine = Engine::default();
                            engine.push(preview.take().ok_or("preview already prepared")?);
                            Ok(OutputEngine {
output_bus:None,

                                input_bus: None,
                                engine,
                                receivers: vec![],
                            })
                        })
                        .map_err(|e| e.to_string())?;
                }
            }
            self.record_project(previous, "Stock/media tools")?;
            self.editor.clear_history();
            Ok(())
        })();
        self.status = match applied {
            Ok(()) => "Media operation completed".into(),
            Err(e) => e.to_string(),
        };
    }
    fn tools_ui(&mut self, root: &mut egui::Ui, action: &mut Option<&str>) {
        root.collapsing("Stock instruments, effects and media",|ui|{
            ui.label("Actions target the selected track; imports use the playback cursor. Stop before structural edits.");
            ui.horizontal(|ui|{
                egui::ComboBox::from_id_salt("stock-instrument").selected_text(stock::INSTRUMENTS[self.tools.synth]).show_ui(ui,|ui|{for (i,n) in stock::INSTRUMENTS.iter().enumerate(){ui.selectable_value(&mut self.tools.synth,i,*n);}});
                if ui.button("Replace instrument").clicked(){*action=Some("tools-synth");}
                egui::ComboBox::from_id_salt("stock-effect").selected_text(stock::EFFECTS[self.tools.effect].0).show_ui(ui,|ui|{for (i,(n,_)) in stock::EFFECTS.iter().enumerate(){ui.selectable_value(&mut self.tools.effect,i,*n);}});
                if ui.button("Add insert").clicked(){*action=Some("tools-effect");}
            });
            ui.label("Live stock effect knobs appear under engine parameters while playing. Synth ADSR and sampler mapping are prepared settings.");
            ui.horizontal(|ui|{ui.label("Directory");ui.text_edit_singleline(&mut self.tools.directory);if ui.button("Refresh / search").clicked(){*action=Some("tools-scan");}
                if ui.button("Parent").clicked(){*action=Some("tools-parent");}});
            ui.horizontal(|ui|{ui.label("Name filter");ui.text_edit_singleline(&mut self.tools.query);if ui.button("Open selected directory").clicked(){*action=Some("tools-open");}});
            egui::ScrollArea::vertical().id_salt("stock-browser").max_height(180.).show(ui,|ui|{for (i,e) in self.tools.entries.iter().enumerate(){ui.selectable_value(&mut self.tools.selected,i,format!("{:?} · {}",e.kind,e.name));}});
            ui.horizontal(|ui|{for (label,a) in [("Audition 30s","tools-preview"),("Import audio","tools-audio"),("Load sampler","tools-sampler"),("Import MIDI","tools-midi"),("Load preset","tools-preset-load")]{if ui.add_enabled(self.tools.job.is_none(),egui::Button::new(label)).clicked(){*action=Some(a);}}});
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.tools.root).range(0..=127).prefix("Sampler root key "));ui.checkbox(&mut self.tools.file_tempo,"Import file tempo over MIDI span");});
            let track=&self.project.document.tracks[self.selected];
            egui::ComboBox::from_id_salt("stock-preset-processor").selected_text(&self.tools.processor).show_ui(ui,|ui|{for d in track.instrument.iter().chain(&track.inserts).filter(|d|d.kind.starts_with("stock.")){ui.selectable_value(&mut self.tools.processor,d.id.clone(),format!("{} · {}",d.id,d.kind));}});
            ui.horizontal(|ui| {for (label,a) in [("Move insert up","tools-insert-up"),("Move insert down","tools-insert-down"),("Remove stock insert","tools-insert-remove")] {if ui.button(label).clicked(){*action=Some(a);}}});
            ui.horizontal(|ui|{ui.label("Preset name");ui.text_edit_singleline(&mut self.tools.preset_name);});
            ui.horizontal(|ui|{ui.label("New preset folder");ui.text_edit_singleline(&mut self.tools.preset_directory);if ui.button("Save selected processor preset").clicked(){*action=Some("tools-preset-save");}});
            if self.tools.job.is_some(){ui.label("Media worker running; changing the project rejects stale imports.");}
        });
    }
}
