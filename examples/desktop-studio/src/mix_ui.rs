#[derive(Default)]
struct MixTools {
    draft: Option<mix_graph::Draft>,
    bus: usize,
    name: String,
    effect: usize,
    source: String,
    link: usize,
    fx: usize,
    master_fx: bool,
}
impl Studio {
    fn mix_action(&mut self, action: &str) -> Result<()> {
        if action == "mix-load" {
            self.stop()?;
            self.mix.draft = Some(mix_graph::Draft::new(&self.project)?);
            self.mix.bus = 0;
            return Ok(());
        }
        if action == "mix-apply" {
            self.stop()?;
            let draft = self.mix.draft.clone().ok_or("load a mixing draft first")?;
            let mut factory = Factory {
                buses:Default::default(),
                mode:soniclaw_host::studio::Mode::Live,
                spec: self.project.document.spec(),
                assets: self.project.assets.clone(),
                states:self.project.native_states.clone(),
                timeline:self.project.document.timeline.clone(),
                trusted: &self.trusted,
                owners: &mut self.owners,
            };
            let result = draft.apply(&mut self.project, &mut factory);
            match result {
                Ok(prepared) => {
                    drop(prepared);
                    self.owners.clear();
                }
                Err(e) => {
                    self.owners.clear();
                    return Err(e);
                }
            }
            self.mix.draft = None;
            return Ok(());
        }
        let draft = self.mix.draft.as_mut().ok_or("load a mixing draft first")?;
        let graph = &mut draft.document.mix_routing;
        match action {
            "mix-add-bus" => {
                if self.mix.name.trim().is_empty() {
                    return Err("name the new submix".into());
                }
                graph.buses.push(mix_graph::Bus::new(
                    workspace::identity("bus")?,
                    self.mix.name.clone(),
                ));
                self.mix.bus = graph.buses.len() - 1;
            }
            "mix-remove-bus" => {
                let id = graph
                    .buses
                    .get(self.mix.bus)
                    .ok_or("select a bus")?
                    .id
                    .clone();
                graph.buses.remove(self.mix.bus);
                for b in &mut graph.buses {
                    if b.output == mix_graph::Destination::Bus(id.clone()) {
                        b.output = mix_graph::Destination::Main;
                    }
                }
                for out in graph.track_outputs.values_mut() {
                    if *out == mix_graph::Destination::Bus(id.clone()) {
                        *out = mix_graph::Destination::Main;
                    }
                }
                self.mix.bus = 0;
            }
            "mix-bus-effect" | "mix-master-effect" => {
                let d = stock::effect(
                    stock::EFFECTS[self.mix.effect].0,
                    &workspace::identity("fx")?,
                )?;
                if action == "mix-master-effect" {
                    graph.master.inserts.push(d);
                } else {
                    graph
                        .buses
                        .get_mut(self.mix.bus)
                        .ok_or("select a submix")?
                        .inserts
                        .push(d);
                }
            }
            "mix-fx-up" | "mix-fx-down" | "mix-fx-remove" => {
                let chain = if self.mix.master_fx {
                    &mut graph.master.inserts
                } else {
                    &mut graph
                        .buses
                        .get_mut(self.mix.bus)
                        .ok_or("select a submix")?
                        .inserts
                };
                let i = self.mix.fx;
                if i >= chain.len() {
                    return Err("insert selection missing".into());
                }
                if action == "mix-fx-remove" {
                    chain.remove(i);
                } else {
                    let to = if action == "mix-fx-up" {
                        i.checked_sub(1)
                    } else {
                        i.checked_add(1)
                    }
                    .filter(|n| *n < chain.len())
                    .ok_or("insert is at chain boundary")?;
                    chain.swap(i, to);
                }
            }
            "mix-add-sidechain" => {
                let target = &self.project.document.tracks[self.selected].id;
                let t = draft
                    .document
                    .tracks
                    .iter_mut()
                    .find(|t| &t.id == target)
                    .ok_or("selected target is absent from draft")?;
                if t.id == self.mix.source {
                    return Err("sidechain source and target must differ".into());
                }
                let d = mix_graph::sidechain_descriptor(workspace::identity("duck")?)?;
                graph.sidechains.push(mix_graph::Sidechain {
                    source: self.mix.source.clone(),
                    target: d.id.clone(),
                });
                t.inserts.push(d);
            }
            "mix-remove-sidechain" => {
                if self.mix.link >= graph.sidechains.len() {
                    return Err("select a sidechain".into());
                }
                let removed = graph.sidechains.remove(self.mix.link);
                for t in &mut draft.document.tracks {
                    t.inserts.retain(|d| d.id != removed.target);
                }
                self.mix.link = 0;
            }
            _ => return Err("unknown mixing action".into()),
        }
        Ok(())
    }
    fn mix_ui(&mut self, root: &mut egui::Ui, action: &mut Option<&str>) {
        root.collapsing("Submixes, sidechains and master chain",|ui|{
            if let Some(c)=&self.control {for m in c.mix_meters(){let peak=m.take_peaks();let maximum=peak[0].max(peak[1]);ui.add(egui::ProgressBar::new(maximum.min(1.)).text(format!("{} {:.1} dBFS",m.id,20.*maximum.max(1e-6).log10())));}}
            if ui.button("Load stopped mixing draft").clicked(){*action=Some("mix-load");}
            let Some(draft)=&mut self.mix.draft else {ui.label("Load a draft to change signal routes. Apply prepares and validates the complete graph.");return;};
            let graph=&mut draft.document.mix_routing;
            ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.mix.name);if ui.button("Add submix").clicked(){*action=Some("mix-add-bus");}});
            let ids:Vec<_>=graph.buses.iter().map(|b|(b.id.clone(),b.name.clone())).collect();
            for t in &draft.document.tracks {
                let output=graph.track_outputs.entry(t.id.clone()).or_default();
                egui::ComboBox::from_id_salt(("track-out",&t.id)).selected_text(format!("{} → {:?}",t.id,output)).show_ui(ui,|ui|{ui.selectable_value(output,mix_graph::Destination::Main,"Main");for (id,name)in &ids{ui.selectable_value(output,mix_graph::Destination::Bus(id.clone()),name);}});
            }
            egui::ComboBox::from_id_salt("submix-select").selected_text(graph.buses.get(self.mix.bus).map_or("Select submix",|b|b.name.as_str())).show_ui(ui,|ui|{for (i,b)in graph.buses.iter().enumerate(){ui.selectable_value(&mut self.mix.bus,i,&b.name);}});
            if let Some(b)=graph.buses.get_mut(self.mix.bus){
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut b.name);ui.add(egui::Slider::new(&mut b.gain,0. ..=4.).text("Gain"));ui.add(egui::Slider::new(&mut b.pan,-1. ..=1.).text("Balance"));ui.checkbox(&mut b.mute,"Mute");ui.checkbox(&mut b.solo,"Solo");});
                egui::ComboBox::from_id_salt("bus-out").selected_text(format!("Output {:?}",b.output)).show_ui(ui,|ui|{ui.selectable_value(&mut b.output,mix_graph::Destination::Main,"Main");for (id,name)in &ids{if id!=&b.id {ui.selectable_value(&mut b.output,mix_graph::Destination::Bus(id.clone()),name);}}});
                if ui.button("Remove selected submix; route children to Main").clicked(){*action=Some("mix-remove-bus");}
            }
            egui::ComboBox::from_id_salt("mix-stock-effect").selected_text(stock::EFFECTS[self.mix.effect].0).show_ui(ui,|ui|{for (i,(name,_))in stock::EFFECTS.iter().enumerate(){ui.selectable_value(&mut self.mix.effect,i,*name);}});
            ui.horizontal(|ui|{if ui.button("Add submix insert").clicked(){*action=Some("mix-bus-effect");}
                if ui.button("Add master insert").clicked(){*action=Some("mix-master-effect");}});
            for (master,chain)in [(false,graph.buses.get(self.mix.bus).map_or(&[][..],|b|&b.inserts)),(true,graph.master.inserts.as_slice())]{for (i,d)in chain.iter().enumerate(){ui.horizontal(|ui|{ui.label(format!("{} {} · {}",if master{"Master"}else{"Submix"},i+1,d.id));for (label,a)in [("↑","mix-fx-up"),("↓","mix-fx-down"),("Remove","mix-fx-remove")]{if ui.button(label).clicked(){self.mix.fx=i;self.mix.master_fx=master;*action=Some(a);}}});}}
            ui.horizontal(|ui|{ui.label("Master");ui.add(egui::Slider::new(&mut graph.master.gain,0. ..=4.).text("Gain"));ui.add(egui::Slider::new(&mut graph.master.pan,-1. ..=1.).text("Balance"));ui.checkbox(&mut graph.master.mute,"Mute");});
            ui.label("Sidechain detector uses source after track inserts, before strip fader/mute. Both source and target chains must declare zero latency.");
            egui::ComboBox::from_id_salt("sidechain-source").selected_text(&self.mix.source).show_ui(ui,|ui|{for t in &draft.document.tracks{ui.selectable_value(&mut self.mix.source,t.id.clone(),&t.id);}});
            if ui.button("Add ducking compressor to selected track").clicked(){*action=Some("mix-add-sidechain");}
            for (i,link)in graph.sidechains.iter().enumerate(){ui.selectable_value(&mut self.mix.link,i,format!("{} → {}",link.source,link.target));}
            if ui.button("Remove selected sidechain and its compressor").clicked(){*action=Some("mix-remove-sidechain");}
            if ui.button("Apply prepared mixing graph").clicked(){*action=Some("mix-apply");}
            ui.label("Cycle/latency/stale-draft errors preserve the current song. Bus/master controls also appear as live engine parameters during playback.");
        });
    }
}
