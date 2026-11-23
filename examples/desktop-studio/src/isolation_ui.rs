#[derive(Default)]
struct IsolationTools {
    helper: String,
    approve: bool,
    dry: bool,
}
impl Studio {
    fn isolation_action(&mut self, action: &str) -> Result<()> {
        use soniclaw_host::studio::isolation_adapter::Policy;
        if action == "approve" {
            if !self.isolation_tools.approve {
                return Err("approve the selected helper executable first".into());
            }
            let helper = PathBuf::from(&self.isolation_tools.helper).canonicalize()?;
            if !helper.is_file() {
                return Err("select a helper executable".into());
            }
            self.trusted.insert(helper);
            self.status =
                "Helper approved for this session. Saved recovery attempts are unchanged.".into();
            return Ok(());
        }
        self.stop()?;
        let id = self.native_tools.instance.clone();
        let mut next = self.project.clone();
        let d = native_state::descriptors_mut(&mut next.document)
            .find(|d| d.id == id)
            .ok_or("select native instance")?;
        if d.kind != "vst3" {
            return Err("Studio isolation currently requires stereo VST3".into());
        }
        match action {
            "enable" => {
                if !self.isolation_tools.approve {
                    return Err("approve the selected helper executable first".into());
                }
                if d.state.get("buses").is_some() {
                    return Err("disconnect multibus routes before stereo isolation".into());
                }
                let helper = PathBuf::from(&self.isolation_tools.helper).canonicalize()?;
                if !helper.is_file() {
                    return Err("select a helper executable".into());
                }
                self.trusted.insert(helper.clone());
                d.state["isolation"] = serde_json::to_value(Policy {
                    helper,
                    timeout_ms: 500,
                    recovery_budget: 3,
                    dry_on_fault: self.isolation_tools.dry,
                })?;
                d.state["isolation_attempts"] = serde_json::json!(0);
                d.state["quarantined"] = serde_json::json!(false);
            }
            "recover" => Policy::request_recovery(d)?,
            "disable" => {
                d.state
                    .as_object_mut()
                    .ok_or("native state object")?
                    .remove("isolation");
                d.state["quarantined"] = serde_json::json!(false);
            }
            _ => return Err("unknown isolation action".into()),
        }
        next.validate()?;
        let old = std::mem::replace(&mut self.project, next);
        self.history
            .record(old, &mut self.project, "Native isolation")?;
        self.editor.clear_history();
        self.status = "Isolation policy saved. Play starts a fresh approved helper.".into();
        Ok(())
    }
    fn isolation_ui(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        ui.collapsing("Native crash isolation", |ui| {
            ui.label("Select an existing stereo VST3 instance in the manager above.");
            ui.text_edit_singleline(&mut self.isolation_tools.helper);
            ui.checkbox(
                &mut self.isolation_tools.approve,
                "I approve this helper executable and its dependencies",
            );
            ui.checkbox(
                &mut self.isolation_tools.dry,
                "Pass delayed dry audio after an effect fails",
            );
            if ui.button("Approve helper for this session").clicked() {
                action = Some("approve")
            }
            ui.horizontal(|ui| {
                if ui.button("Enable / reset budget").clicked() {
                    action = Some("enable")
                }
                if ui.button("Recover instance").clicked() {
                    action = Some("recover")
                }
                if ui.button("Disable isolation").clicked() {
                    action = Some("disable")
                }
            });
            for n in &self.owners {
                if let Some(pid) = n.isolated_pid() {
                    ui.label(format!(
                        "{}: helper {pid} · {}",
                        n.id,
                        n.fault.as_deref().unwrap_or("running")
                    ));
                }
            }
            for d in native_state::descriptors(&self.project.document) {
                if d.state["quarantined"].as_bool() == Some(true) {
                    ui.label(format!(
                        "{} quarantined: {}",
                        d.id,
                        d.state["last_native_error"]
                            .as_str()
                            .unwrap_or("helper failed")
                    ));
                }
            }
        });
        if let Some(a) = action {
            if let Err(e) = self.isolation_action(a) {
                self.status = e.to_string();
            }
        }
    }
}
