#[derive(Default)]
struct CueTools {
    draft: Option<mix_graph::Draft>,
    selected: usize,
    name: String,
    kind: usize,
    source: String,
    channels: [u16; 2],
    aux: usize,
    send: usize,
}
impl Studio {
    fn cues_action(&mut self, action: &str) -> Result<()> {
        if action == "cues-load" {
            self.stop()?;
            self.cues.draft = Some(mix_graph::Draft::new(&self.project)?);
            self.cues.selected = 0;
            return Ok(());
        }
        if action == "cues-apply" {
            self.stop()?;
            let draft = self.cues.draft.clone().ok_or("load a cue draft first")?;
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
            match draft.apply(&mut self.project, &mut factory) {
                Ok(p) => {
                    drop(p);
                    self.owners.clear();
                }
                Err(e) => {
                    self.owners.clear();
                    return Err(e);
                }
            }
            self.cues.draft = None;
            return Ok(());
        }
        let draft = self.cues.draft.as_mut().ok_or("load a cue draft first")?;
        match action {
            "cues-add" => {
                if draft.document.cues.len() >= 4 {
                    return Err("at most four cue mixes".into());
                }
                let i = draft.document.cues.len();
                let main = self
                    .manager
                    .preferences()
                    .duplex
                    .as_ref()
                    .map_or([0, 1], |r| r.output_channels);
                let pair = (0..64u16)
                    .step_by(2)
                    .map(|n| [n, n + 1])
                    .find(|p| {
                        p.iter().all(|ch| {
                            !main.contains(ch)
                                && !draft.document.cues.iter().any(|c| c.outputs.contains(ch))
                        })
                    })
                    .ok_or("no unused output pair")?;
                let name = if self.cues.name.trim().is_empty() {
                    format!("Cue {}", i + 1)
                } else {
                    self.cues.name.clone()
                };
                draft
                    .document
                    .cues
                    .push(cues::Cue::new(workspace::identity("cue")?, name, pair));
                self.cues.selected = i;
            }
            "cues-remove" => {
                if self.cues.selected >= draft.document.cues.len() {
                    return Err("select a cue".into());
                }
                draft.document.cues.remove(self.cues.selected);
                self.cues.selected = 0;
            }
            "cues-send-add" => {
                let source = match self.cues.kind {
                    0 => cues::Source::TrackPre(self.cues.source.clone()),
                    1 => cues::Source::TrackPost(self.cues.source.clone()),
                    2 => cues::Source::Bus(self.cues.source.clone()),
                    3 => cues::Source::Aux(self.cues.aux),
                    4 => cues::Source::Main,
                    5 => cues::Source::Input(vec![self.cues.channels[0]]),
                    _ => cues::Source::Input(self.cues.channels.to_vec()),
                };
                draft
                    .document
                    .cues
                    .get_mut(self.cues.selected)
                    .ok_or("select a cue")?
                    .sends
                    .push(cues::Send {
                        id: workspace::identity("send")?,
                        source,
                        gain: 1.,
                        pan: 0.,
                    });
            }
            "cues-send-remove" => {
                let c = draft
                    .document
                    .cues
                    .get_mut(self.cues.selected)
                    .ok_or("select a cue")?;
                if self.cues.send >= c.sends.len() {
                    return Err("select a cue send".into());
                }
                c.sends.remove(self.cues.send);
            }
            _ => return Err("unknown cue action".into()),
        }
        Ok(())
    }
    fn cues_ui(&mut self, root: &mut egui::Ui, action: &mut Option<&str>) {
        root.collapsing("Headphone cue mixes and hardware outputs",|ui|{
            if let Some(c)=&self.control{for (id,frames)in c.cue_latencies(){ui.label(format!("{id}: maximum source delay {frames} frames"));}}
            if ui.button("Load stopped cue draft").clicked(){*action=Some("cues-load");}
            let Some(draft)=&mut self.cues.draft else {ui.label("Create independent monitor mixes, then choose non-overlapping physical output pairs. Raw input taps are silent until audio input is enabled.");return;};
            ui.horizontal(|ui|{ui.text_edit_singleline(&mut self.cues.name);if ui.button("Add cue").clicked(){*action=Some("cues-add");}});
            egui::ComboBox::from_id_salt("cue-select").selected_text(draft.document.cues.get(self.cues.selected).map_or("Select cue",|c|c.name.as_str())).show_ui(ui,|ui|{for (i,c)in draft.document.cues.iter().enumerate(){ui.selectable_value(&mut self.cues.selected,i,&c.name);}});
            if let Some(c)=draft.document.cues.get_mut(self.cues.selected){
                ui.horizontal(|ui|{ui.text_edit_singleline(&mut c.name);ui.add(egui::Slider::new(&mut c.gain,0. ..=4.).text("Cue gain"));ui.add(egui::Slider::new(&mut c.pan,-1. ..=1.).text("Balance"));ui.checkbox(&mut c.mute,"Mute");});
                ui.horizontal(|ui|{ui.label("Physical output indices (0-based)");ui.add(egui::DragValue::new(&mut c.outputs[0]).range(0..=63).prefix("L "));ui.add(egui::DragValue::new(&mut c.outputs[1]).range(0..=63).prefix("R "));ui.checkbox(&mut c.align_sources,"Align source delays");ui.checkbox(&mut c.metronome,"Recording click");});
                ui.label("Track Pre taps ignore strip mute/solo/fader; Post taps follow them. Raw inputs provide the shortest software path. The interface/OS still adds latency.");
                egui::ComboBox::from_id_salt("cue-source-kind").selected_text(["Track Pre","Track Post","Submix","Aux return","Main mix","Mono input","Stereo input"][self.cues.kind]).show_ui(ui,|ui|{for (i,n)in ["Track Pre","Track Post","Submix","Aux return","Main mix","Mono input","Stereo input"].iter().enumerate(){ui.selectable_value(&mut self.cues.kind,i,*n);}});
                if self.cues.kind<=2 {egui::ComboBox::from_id_salt("cue-source-id").selected_text(&self.cues.source).show_ui(ui,|ui|{if self.cues.kind==2 {for b in &draft.document.mix_routing.buses{ui.selectable_value(&mut self.cues.source,b.id.clone(),&b.name);}}else{for t in &draft.document.tracks{ui.selectable_value(&mut self.cues.source,t.id.clone(),&t.id);}}});}
                if self.cues.kind==3 {ui.add(egui::DragValue::new(&mut self.cues.aux).range(0..=3).prefix("Aux index "));}
                if self.cues.kind>=5 {ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.cues.channels[0]).range(0..=63).prefix("Input L "));if self.cues.kind==6 {ui.add(egui::DragValue::new(&mut self.cues.channels[1]).range(0..=63).prefix("Input R "));}});}
                if ui.button("Add source to cue").clicked(){*action=Some("cues-send-add");}
                for (i,s)in c.sends.iter_mut().enumerate(){ui.horizontal(|ui|{ui.label(format!("{:?}",s.source));ui.add(egui::Slider::new(&mut s.gain,0. ..=4.).text("Send gain"));ui.add(egui::Slider::new(&mut s.pan,-1. ..=1.).text("Balance"));if ui.button("Remove send").clicked(){self.cues.send=i;*action=Some("cues-send-remove");}});}
                if ui.button("Remove selected cue").clicked(){*action=Some("cues-remove");}
            }
            if ui.button("Apply prepared cue mixes").clicked(){*action=Some("cues-apply");}
            ui.label("Main and cue output pairs must be distinct and supported by the selected device. Play negotiates the required hardware channel count. Save retains cue routes in the song.");
        });
    }
}
