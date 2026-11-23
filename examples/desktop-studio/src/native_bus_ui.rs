#[derive(Default)]
struct NativeBusTools {
    request: Option<native_buses::Request>,
    source: String,
    input: usize,
    output: usize,
    mono: bool,
    name: String,
    product: String,
    path: PathBuf,
}
impl Studio {
    fn native_bus_action(&mut self, action: &str) -> Result<()> {
        if action == "inspect" {
            if !self.native_tools.trust {
                return Err("approve selected module before port inspection".into());
            }
            let e = self
                .native_tools
                .registry
                .usable(self.native_tools.selected)?
                .clone();
            if e.product.format != soniclaw_host::formats::PluginFormat::Vst3 {
                return Err("Studio multibus owner currently requires VST3; CLAP multibus is available through the framework's bus/isolation APIs".into());
            }
            self.stop()?;
            // SAFETY: selected native module was explicitly approved, and registry checks its fingerprint.
            let loader = unsafe {
                soniclaw_host::formats::TrustedLoader::new(std::slice::from_ref(&e.product.path))
            }?;
            let p = loader.load_vst3_buses(&e.product, self.project.document.spec())?;
            self.native_buses.request = Some(native_buses::Request {
                layout: p.layout().clone(),
                sidechains: Default::default(),
            });
            self.native_buses.product = e.product.id;
            self.native_buses.path = e.product.path;
            self.native_buses.input = 1;
            self.native_buses.output = 1;
            return Ok(());
        }
        self.stop()?;
        let id = self.native_tools.instance.clone();
        let mut next = self.project.clone();
        let desc = native_state::descriptors_mut(&mut next.document)
            .find(|d| d.id == id)
            .ok_or("select an existing native track instance")?;
        match action {
            "enable" => {
                let request = self
                    .native_buses
                    .request
                    .clone()
                    .ok_or("inspect selected product ports first")?;
                if desc.kind != "vst3"
                    || desc.state["class_id"].as_str() != Some(&self.native_buses.product)
                    || desc.state["path"].as_str().map(Path::new)
                        != Some(self.native_buses.path.as_path())
                {
                    return Err("inspected ports do not belong to selected native instance".into());
                }
                request.store(desc)?;
            }
            "input" | "disconnect" => {
                let mut request = native_buses::Request::from_descriptor(desc)?
                    .ok_or("enable native buses first")?;
                if action == "input" {
                    request
                        .sidechains
                        .insert(self.native_buses.input, self.native_buses.source.clone());
                } else {
                    request.sidechains.remove(&self.native_buses.input);
                }
                request.store(desc)?;
            }
            "return" => {
                let request = native_buses::Request::from_descriptor(desc)?
                    .ok_or("enable native buses first")?;
                let shape = request
                    .layout
                    .outputs
                    .get(self.native_buses.output)
                    .ok_or("select valid output bus")?;
                let channels = if self.native_buses.mono || shape.channels == 1 {
                    vec![0]
                } else {
                    vec![0, 1]
                };
                let track = workspace::identity("return")?;
                workspace::apply(
                    &mut next,
                    workspace::Command::Add {
                        id: track.clone(),
                        name: if self.native_buses.name.is_empty() {
                            format!("{} bus {}", id, self.native_buses.output)
                        } else {
                            self.native_buses.name.clone()
                        },
                        kind: workspace::Kind::Audio,
                    },
                )?;
                let t = next
                    .document
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == track)
                    .expect("added");
                t.instrument = Some(
                    native_buses::Return {
                        processor: id,
                        bus: self.native_buses.output,
                        channels,
                    }
                    .descriptor(&workspace::identity("port")?)?,
                );
            }
            _ => return Err("unknown native bus action".into()),
        }
        next.validate()?;
        let previous = std::mem::replace(&mut self.project, next);
        self.history
            .record(previous, &mut self.project, "Native bus routing")?;
        self.editor.clear_history();
        Ok(())
    }
    fn native_bus_ui(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.collapsing("Native sidechains and output returns",|ui|{
  ui.label("Select a trusted product and native instance in the plug-in manager. Bus routing is prepared while stopped.");
  if ui.button("Inspect selected VST3 ports").clicked(){action=Some("inspect");}
  if let Some(r)=&self.native_buses.request{for(i,b)in r.layout.inputs.iter().enumerate(){ui.label(format!("Input {i}: {} ({} channels)",b.name,b.channels));}for(i,b)in r.layout.outputs.iter().enumerate(){ui.label(format!("Output {i}: {} ({} channels)",b.name,b.channels));}}
  if ui.button("Enable inspected buses on selected instance").clicked(){action=Some("enable");}
  ui.horizontal(|ui|{ui.label("Sidechain input bus");ui.add(egui::DragValue::new(&mut self.native_buses.input).range(1..=7));});
  egui::ComboBox::from_id_salt("native-key-source").selected_text(&self.native_buses.source).show_ui(ui,|ui|{for t in &self.project.document.tracks{ui.selectable_value(&mut self.native_buses.source,t.id.clone(),&t.id);}});
  if ui.button("Connect sidechain").clicked(){action=Some("input");}
  if ui.button("Disconnect sidechain input").clicked(){action=Some("disconnect");}
  ui.horizontal(|ui|{ui.label("Output bus");ui.add(egui::DragValue::new(&mut self.native_buses.output).range(0..=7));ui.checkbox(&mut self.native_buses.mono,"Mono return");});ui.text_edit_singleline(&mut self.native_buses.name);
  if ui.button("Add independent output-return track").clicked(){action=Some("return");}
  ui.label("Sidechain source and native insert prefix currently require zero latency. Returns are independent mixer tracks; mute the source's main strip if appropriate. Disconnect linked routes before freezing/deleting their source.");
 });
        if let Some(a) = action {
            if let Err(e) = self.native_bus_action(a) {
                self.status = e.to_string();
            }
        }
    }
}
