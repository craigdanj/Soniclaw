#[derive(Default)]
struct NativeTools {
    module: String,
    cache: String,
    vst3: bool,
    trust: bool,
    instrument: bool,
    parameters_only: bool,
    automation_ids:String,
    selected: usize,
    target: usize,
    instance: String,
    registry: soniclaw_host::studio::registry::Registry,
    scan: Option<
        std::sync::mpsc::Receiver<
            std::result::Result<soniclaw_host::studio::registry::Report, String>,
        >,
    >,
}
impl Studio {
    fn native_action(&mut self, action: &str) -> Result<()> {
        use soniclaw_host::studio::registry;
        match action {
            "scan" => {
                if !self.native_tools.trust {
                    return Err(
                        "approve the selected module and its dependencies before scanning".into(),
                    );
                }
                if self.native_tools.scan.is_some() {
                    return Err("scan already running".into());
                }
                let module = PathBuf::from(&self.native_tools.module);
                let helper = std::env::current_exe()?;
                let format = if self.native_tools.vst3 {
                    soniclaw_host::formats::PluginFormat::Vst3
                } else {
                    soniclaw_host::formats::PluginFormat::Clap
                };
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    // SAFETY: user explicitly approved this selected native module; helper is this executable.
                    let r = unsafe {
                        registry::scan(&helper, &module, format, Duration::from_secs(10))
                    }
                    .map_err(|e| e.to_string());
                    let _ = tx.send(r);
                });
                self.native_tools.scan = Some(rx);
            }
            "save" => self
                .native_tools
                .registry
                .write(Path::new(&self.native_tools.cache))?,
            "load" => {
                self.native_tools.registry =
                    registry::Registry::read(Path::new(&self.native_tools.cache))?;
                self.native_tools.trust = false;
            }
            "install" | "repair" => {
                if !self.native_tools.trust {
                    return Err("explicitly approve module before loading or repair".into());
                }
                let e = self
                    .native_tools
                    .registry
                    .usable(self.native_tools.selected)?
                    .clone();
                self.stop()?;
                let mut candidate = self.project.clone();
                if action == "repair" {
                    let id = self.native_tools.instance.clone();
                    if candidate.freezes.values().any(|f|native_state::descriptors(&f.original).any(|d|d.id==id)){return Err("unfreeze affected tracks before native path repair".into());}
                    let d = native_state::descriptors_mut(&mut candidate.document)
                        .find(|d| d.id == id)
                        .ok_or("select an existing native instance")?;
                    let new = registry::descriptor(
                        &e,
                        &id,
                        d.state["role"].as_str() == Some("instrument"),
                    );
                    if d.kind != new.kind
                        || d.state[if d.kind == "clap" {
                            "plugin_id"
                        } else {
                            "class_id"
                        }] != new.state[if d.kind == "clap" {
                            "plugin_id"
                        } else {
                            "class_id"
                        }]
                    {
                        return Err("repair requires the same native format and product ID".into());
                    }
                    let automation_ids=d.state.get("automation_ids").cloned();
                    let saved = d.state["native_state"].clone();
                    let latency = d.state["latency_frames"].clone();
                    let state_mode=d.state["state_mode"].clone();
                    *d = new;
                    d.state["native_state"] = saved;
                    if let Some(ids)=automation_ids{d.state["automation_ids"]=ids;}
                    d.state["latency_frames"] = latency;
                    d.state["state_mode"]=state_mode;
                    if let Some(blob) = candidate.native_states.get(&id).cloned() {
                        candidate
                            .native_states
                            .insert(id, blob.rebind(e.product.path.clone())?);
                    }
                } else {
                    let id = workspace::identity("native")?;
                    let mut d = registry::descriptor(&e, &id, self.native_tools.instrument);
                    if !self.native_tools.automation_ids.trim().is_empty() {let ids=self.native_tools.automation_ids.split(',').map(|s|s.trim().parse::<u32>()).collect::<std::result::Result<Vec<_>,_>>()?;if ids.len()>64{return Err("select at most 64 native IDs".into());}d.state["automation_ids"]=serde_json::json!(ids);}
                    if self.native_tools.parameters_only {
                        d.state["state_mode"] = serde_json::json!("parameters");
                    }
                    let tracks = candidate.document.tracks.len();
                    if self.native_tools.target < tracks {
                        let t = &mut candidate.document.tracks[self.native_tools.target];
                        if self.native_tools.instrument {
                            if t.instrument.is_some() {
                                return Err("remove existing instrument first".into());
                            }
                            t.instrument = Some(d);
                        } else {
                            t.inserts.push(d);
                        }
                    } else {
                        if self.native_tools.instrument {
                            return Err("instruments require a track".into());
                        }
                        let b = self.native_tools.target - tracks;
                        if b < candidate.document.mix_routing.buses.len() {
                            candidate.document.mix_routing.buses[b].inserts.push(d);
                        } else {
                            candidate.document.mix_routing.master.inserts.push(d);
                        }
                    }
                }
                candidate.validate()?;
                let previous = std::mem::replace(&mut self.project, candidate);
                self.history
                    .record(previous, &mut self.project, "Native plug-in assignment")?;
                self.trusted.insert(e.product.path);
                self.editor.clear_history();
                self.status = "Native assignment saved; Play constructs the instance".into();
            }
            "bypass" => {
                self.stop()?;
                let mut next = self.project.clone();
                let d = native_state::descriptors_mut(&mut next.document)
                    .find(|d| {
                        d.id == self.native_tools.instance
                            && matches!(d.kind.as_str(), "clap" | "vst3")
                    })
                    .ok_or("select a native instance ID")?;
                let bypass = !d.state["bypass"].as_bool().unwrap_or(false);
                d.state["bypass"] = serde_json::json!(bypass);
                next.validate()?;
                let previous = std::mem::replace(&mut self.project, next);
                self.history
                    .record(previous, &mut self.project, "Native bypass")?;
            }
            _ => return Err("unknown native action".into()),
        }
        Ok(())
    }
    fn native_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(rx) = &self.native_tools.scan {
            match rx.try_recv() {
                Ok(r) => {
                    self.native_tools.scan = None;
                    match r {
                        Ok(report) => {
                            let status = report.error.clone().unwrap_or_else(|| {
                                format!("Discovered {} products", report.products.len())
                            });
                            match self.native_tools.registry.apply(report) {
                                Ok(()) => self.status = status,
                                Err(e) => self.status = e.to_string(),
                            }
                        }
                        Err(e) => self.status = e,
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.native_tools.scan = None;
                    self.status = "Scan worker disconnected".into();
                }
                Err(_) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            }
        }
        let mut action = None;
        ui.collapsing("Native plug-in manager (CLAP / VST3)",|ui|{
   ui.label("Scan a selected trusted module. Scans run in a supervised process; live native playback runs in this application.");
   ui.horizontal(|ui|{ui.label("Module");ui.text_edit_singleline(&mut self.native_tools.module);ui.checkbox(&mut self.native_tools.vst3,"VST3");});
   ui.checkbox(&mut self.native_tools.trust,"I trust the selected module and its dependencies");
   if ui.add_enabled(self.native_tools.scan.is_none(),egui::Button::new("Scan module")).clicked(){action=Some("scan");}
   ui.horizontal(|ui|{ui.label("Cache file");ui.text_edit_singleline(&mut self.native_tools.cache);if ui.button("Save cache").clicked(){action=Some("save");}
if ui.button("Load cache").clicked(){action=Some("load");}});
   for (i,e) in self.native_tools.registry.entries.iter_mut().enumerate(){ui.horizontal(|ui|{ui.checkbox(&mut e.enabled,"Enabled");ui.selectable_value(&mut self.native_tools.selected,i,format!("{} / {:?} / {}",e.product.name,e.product.format,e.product.id));});}
   for(p,f)in &self.native_tools.registry.quarantine{ui.label(format!("Quarantined {}: {} — rescan to retry",p.display(),f.reason));}
   let labels=self.project.document.tracks.iter().map(|t|format!("Track {}",t.id)).chain(self.project.document.mix_routing.buses.iter().map(|b|format!("Bus {}",b.name))).chain(std::iter::once("Master".into())).collect::<Vec<_>>();
   egui::ComboBox::from_id_salt("native-target").selected_text(labels.get(self.native_tools.target).cloned().unwrap_or_default()).show_ui(ui,|ui|{for(i,l)in labels.iter().enumerate(){ui.selectable_value(&mut self.native_tools.target,i,l);}});
   ui.checkbox(&mut self.native_tools.instrument,"Install as track instrument");ui.checkbox(&mut self.native_tools.parameters_only,"Parameter-only checkpoint (for products without opaque state)");
            ui.label("Automation IDs: comma-separated writable native IDs; blank selects all (maximum 64)");ui.text_edit_singleline(&mut self.native_tools.automation_ids);
            if ui.button("Install selected product").clicked(){action=Some("install");}
   egui::ComboBox::from_id_salt("native-instance").selected_text(&self.native_tools.instance).show_ui(ui,|ui|{for d in native_state::descriptors(&self.project.document).filter(|d|matches!(d.kind.as_str(),"clap"|"vst3")){ui.selectable_value(&mut self.native_tools.instance,d.id.clone(),format!("{} / {}{}",d.id,d.kind,if d.state["bypass"].as_bool()==Some(true){" (bypassed)"}else{""}));}});
   ui.horizontal(|ui|{if ui.button("Toggle bypass").clicked(){action=Some("bypass");}
if ui.button("Repair path with selected product").clicked(){action=Some("repair");}});
  });
        if let Some(a) = action {
            if let Err(e) = self.native_action(a) {
                self.status = e.to_string();
            }
        }
    }
}
