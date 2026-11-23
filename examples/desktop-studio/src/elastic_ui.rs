struct ElasticTools {
    source: String,
    group: Option<elastic::Group>,
    detection: elastic::Detection,
    grid: f64,
    onsets: Vec<u64>,
    analysis: Option<jobs::Ticket<(elastic::Group, Vec<u64>)>>,
    render: Option<jobs::Ticket<elastic::Ready>>,
    directory: String,
    saved: String,
}
impl Default for ElasticTools {
    fn default() -> Self {
        Self {
            source: String::new(),
            group: None,
            detection: Default::default(),
            grid: 0.25,
            onsets: vec![],
            analysis: None,
            render: None,
            directory: String::new(),
            saved: String::new(),
        }
    }
}
impl Studio {
    fn elastic_action(&mut self, action: &str) -> Result<()> {
        match action {
            "elastic-new" => {
                self.elastic.group = None;
                self.elastic.onsets.clear();
            }
            "elastic-add" => {
                self.stop()?;
                let source = self.elastic.source.clone();
                let path = self
                    .project
                    .disk
                    .get(&source)
                    .ok_or("select an original WAV")?;
                let r = hound::WavReader::open(path)?;
                let frames = r.duration() as u64;
                let track = &self.project.document.tracks[self.selected];
                let replace = track
                    .streamed
                    .iter()
                    .find(|c| {
                        c.asset == source
                            && c.enabled
                            && !c.looping
                            && c.source_offset == 0
                            && c.source_length == frames
                    })
                    .map(|c| c.id.clone());
                if replace.is_none()
                    && track
                        .streamed
                        .iter()
                        .any(|c| c.asset == source && c.enabled)
                {
                    return Err("existing original must be a whole unlooped source; use another track or render it first".into());
                }
                let m = elastic::Member {
                    source,
                    track: track.id.clone(),
                    region: workspace::identity("elastic")?,
                    asset: workspace::identity("cache")?,
                    replace,
                };
                let mut group = if let Some(g) = &self.elastic.group {
                    g.clone()
                } else {
                    let map =
                        soniclaw::transport::TempoMap::compile(&self.project.document.timeline)?;
                    let start = map.sample_at(
                        self.play_from_beat,
                        self.project.document.sample_rate as f64,
                    )?;
                    let end = start
                        + (frames as f64 * self.project.document.sample_rate as f64
                            / r.spec().sample_rate as f64)
                            .round() as u64;
                    elastic::Group {
                        id: workspace::identity("elastic")?,
                        reference: 0,
                        members: vec![],
                        recipe: tempo_cache::Recipe {
                            schema: 1,
                            mode: tempo_cache::Mode::PreservePitch,
                            markers: vec![
                                tempo_cache::Marker {
                                    source_frame: 0,
                                    beat: self.play_from_beat,
                                },
                                tempo_cache::Marker {
                                    source_frame: frames,
                                    beat: map.beat_at_sample(
                                        end,
                                        self.project.document.sample_rate as f64,
                                    )?,
                                },
                            ],
                        },
                    }
                };
                group.members.push(m);
                group.validate(&self.project)?;
                self.elastic.group = Some(group);
                self.elastic.onsets.clear();
            }
            "elastic-detect" => {
                let g = self
                    .elastic
                    .group
                    .as_ref()
                    .ok_or("add elastic members first")?;
                g.validate(&self.project)?;
                let path = self.project.disk[&g.members[g.reference].source].clone();
                let analyzed_group = g.clone();
                let settings = self.elastic.detection;
                self.elastic.analysis = Some(self.jobs.submit(
                    "elastic-detect",
                    self.job_epoch,
                    jobs::Priority::Interactive,
                    192 * 1024 * 1024,
                    move |_| {
                        elastic::detect(&path, settings)
                            .map(|v| (analyzed_group, v))
                            .map_err(|e| e.to_string())
                    },
                )?);
            }
            "elastic-quantize" => {
                self.elastic
                    .group
                    .as_mut()
                    .ok_or("add elastic members first")?
                    .quantize(&self.project, &self.elastic.onsets, self.elastic.grid)?;
            }
            "elastic-render" => {
                self.stop()?;
                if self.elastic.render.is_some() {
                    return Err("wait for or cancel elastic render".into());
                }
                let g = self
                    .elastic
                    .group
                    .clone()
                    .ok_or("add elastic members first")?;
                g.validate(&self.project)?;
                let p = self.project.clone();
                let dir = PathBuf::from(&self.elastic.directory);
                self.elastic.render = Some(self.jobs.submit(
                    "elastic-render",
                    self.job_epoch,
                    jobs::Priority::Normal,
                    64 * 1024 * 1024,
                    move |ctx| {
                        g.build(&p, &dir, &ctx.cancel_flag())
                            .map_err(|e| e.to_string())
                    },
                )?);
            }
            "elastic-load" => {
                self.elastic.group = Some(
                    self.project
                        .layout
                        .elastic_groups
                        .iter()
                        .find(|g| g.id == self.elastic.saved)
                        .ok_or("select a saved elastic group")?
                        .clone(),
                );
                self.elastic.onsets.clear();
            }
            "elastic-dissolve" => {
                self.stop()?;
                elastic::dissolve(&mut self.project, &self.elastic.saved)?;
            }
            _ => return Err("unknown elastic action".into()),
        }
        Ok(())
    }
    fn poll_elastic(&mut self) {
        if let Some(r) = self
            .elastic
            .analysis
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.elastic.analysis = None;
            match r {
                Ok((group, onsets)) => {
                    if self.elastic.group.as_ref() != Some(&group) {
                        self.status =
                            "Elastic draft changed during onset detection; run detection again"
                                .into();
                        return;
                    }
                    self.status = format!(
                        "{} onset candidates; inspect and quantize anchors",
                        onsets.len()
                    );
                    self.elastic.onsets = onsets;
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(r) = self
            .elastic
            .render
            .as_mut()
            .and_then(|t| t.poll(self.job_epoch))
        {
            self.elastic.render = None;
            let previous = self.project.clone();
            let result = (|| -> Result<()> {
                if self.control.is_some() || self.recording {
                    return Err(
                        "stop playback/recording and rebuild before applying elastic edits".into(),
                    );
                }
                r?.apply(&mut self.project)?;
                self.record_project(previous, "Phase-linked elastic edit")?;
                self.editor.clear_history();
                Ok(())
            })();
            self.status = match result {
                Ok(()) => "Shared elastic caches verified and applied".into(),
                Err(e) => e.to_string(),
            };
        }
    }
    fn elastic_ui(&mut self, root: &mut egui::Ui, action: &mut Option<&str>) {
        root.collapsing("Transient and phase-linked elastic editing",|ui|{
            ui.label("Select an original disk WAV and destination track, then Add member. Originals must share rate/length. Existing whole-file original regions are disabled on commit.");
            egui::ComboBox::from_id_salt("elastic-source").selected_text(&self.elastic.source).show_ui(ui,|ui|{for id in self.project.disk.keys().filter(|id|!self.project.warps.contains_key(*id)){ui.selectable_value(&mut self.elastic.source,id.clone(),id);}});
            ui.horizontal(|ui|{if ui.button("New group").clicked(){*action=Some("elastic-new");}
                if ui.button("Add source to selected track").clicked(){*action=Some("elastic-add");}});
            if let Some(g)=&mut self.elastic.group{
                for (i,m) in g.members.iter().enumerate(){ui.radio_value(&mut g.reference,i,format!("Reference: {} → {}",m.source,m.track));}
                ui.horizontal(|ui|{ui.selectable_value(&mut g.recipe.mode,tempo_cache::Mode::PreservePitch,"Preserve pitch");ui.selectable_value(&mut g.recipe.mode,tempo_cache::Mode::Repitch,"Repitch");});
                egui::ScrollArea::vertical().id_salt("elastic-markers").max_height(150.).show(ui,|ui|{for m in &mut g.recipe.markers {ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut m.source_frame).prefix("Source frame "));ui.add(egui::DragValue::new(&mut m.beat).range(0. ..=1000000.).prefix("Target beat "));});}});
            }
            ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.elastic.detection.threshold).range(1.1..=20.).prefix("Onset ratio "));ui.add(egui::DragValue::new(&mut self.elastic.detection.floor_db).range(-80. ..= -6.).prefix("Floor dB "));ui.add(egui::DragValue::new(&mut self.elastic.detection.gap_ms).range(10. ..=1000.).prefix("Gap ms "));});
            ui.horizontal(|ui|{if ui.add_enabled(self.elastic.analysis.is_none(),egui::Button::new("Detect reference onsets")).clicked(){*action=Some("elastic-detect");}ui.add(egui::DragValue::new(&mut self.elastic.grid).range(1./64. ..=4.).prefix("Grid beats "));if ui.button("Replace interior anchors with snapped onsets").clicked(){*action=Some("elastic-quantize");}});
            ui.label(format!("{} source onset candidates",self.elastic.onsets.len()));
            ui.horizontal(|ui|{ui.label("New cache folder");ui.text_edit_singleline(&mut self.elastic.directory);if ui.add_enabled(self.elastic.render.is_none(),egui::Button::new("Render and apply whole group")).clicked(){*action=Some("elastic-render");}});
            if let Some(job)=&self.elastic.render{if ui.button("Cancel group render").clicked(){job.cancel();}}
            egui::ComboBox::from_id_salt("saved-elastic-group").selected_text(&self.elastic.saved).show_ui(ui,|ui|{for g in &self.project.layout.elastic_groups{ui.selectable_value(&mut self.elastic.saved,g.id.clone(),&g.id);}});
            ui.horizontal(|ui|{if ui.button("Load saved group draft").clicked(){*action=Some("elastic-load");}
                if ui.button("Dissolve shared edits; keep audio").clicked(){*action=Some("elastic-dissolve");}});
        });
    }
}
