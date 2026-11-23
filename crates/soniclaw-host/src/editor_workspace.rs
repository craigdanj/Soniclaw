//! Persistable editor preferences for already-loaded trusted instances. Never loads modules itself.
use crate::{
    live::{LiveRack, RackEntry},
    HostError,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Owned,
    Floating,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub module: PathBuf,
    pub plugin_id: String,
    pub mode: Mode,
    pub size: Option<[u32; 2]>,
    pub scale: f64,
    pub reopen: bool,
}
impl State {
    pub fn new(entry: &RackEntry) -> Self {
        Self {
            module: entry.path.clone(),
            plugin_id: entry.plugin_id.clone(),
            mode: Mode::Owned,
            size: None,
            scale: 1.,
            reopen: false,
        }
    }
    pub fn validate(&self) -> Result<(), HostError> {
        if self.module.as_os_str().is_empty()
            || self.module.as_os_str().len() > 4096
            || self.plugin_id.is_empty()
            || self.plugin_id.len() > 256
            || !self.scale.is_finite()
            || !(0.5..=4.).contains(&self.scale)
            || self
                .size
                .is_some_and(|s| s.iter().any(|n| !(64..=8192).contains(n)))
        {
            return Err("invalid plug-in window preferences".into());
        }
        Ok(())
    }
    /// Apply only to an instance whose module and product identity were independently trusted and loaded.
    pub fn open(
        &self,
        rack: &mut LiveRack,
        node: usize,
        title: &str,
        entry: &RackEntry,
    ) -> Result<Vec<String>, HostError> {
        self.validate()?;
        if self.module != entry.path || self.plugin_id != entry.plugin_id {
            return Err("window preferences belong to another plug-in".into());
        }
        match self.mode {
            Mode::Owned => rack.show_embedded_editor(node, title)?,
            Mode::Floating => rack.show_editor(node)?,
        }
        let mut warnings = vec![];
        if self.scale != 1. {
            if let Err(e) = rack.scale_editor(node, self.scale) {
                warnings.push(e.to_string());
            }
        }
        if let Some(size) = self.size {
            if let Err(e) = rack.resize_editor(node, size) {
                warnings.push(e.to_string());
            }
        }
        Ok(warnings)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub schema: u32,
    pub windows: BTreeMap<String, State>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            schema: 1,
            windows: BTreeMap::new(),
        }
    }
}
impl Workspace {
    pub fn validate(&self) -> Result<(), HostError> {
        if self.schema != 1
            || self.windows.len() > 256
            || self.windows.keys().any(|k| k.is_empty() || k.len() > 128)
        {
            return Err("invalid window workspace".into());
        }
        for s in self.windows.values() {
            s.validate()?;
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, HostError> {
        self.validate()?;
        let s = serde_json::to_string_pretty(self)?;
        if s.len() > 262144 {
            return Err("window workspace exceeds 256 KiB".into());
        }
        Ok(s)
    }
    pub fn from_json(s: &str) -> Result<Self, HostError> {
        if s.len() > 262144 {
            return Err("window workspace exceeds 256 KiB".into());
        }
        let w: Self = serde_json::from_str(s)?;
        w.validate()?;
        Ok(w)
    }
    pub fn for_instance(&mut self, id: &str, entry: &RackEntry) -> Result<&mut State, HostError> {
        if id.is_empty()
            || id.len() > 128
            || !self.windows.contains_key(id) && self.windows.len() >= 256
        {
            return Err("window workspace identity/capacity".into());
        }
        let state = self
            .windows
            .entry(id.into())
            .or_insert_with(|| State::new(entry));
        if state.module != entry.path || state.plugin_id != entry.plugin_id {
            *state = State::new(entry);
        }
        Ok(state)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_is_bounded_and_identity_specific() {
        let entry = RackEntry {
            path: PathBuf::from("/trusted/synth.clap"),
            plugin_id: "org.demo.synth".into(),
        };
        let mut w = Workspace::default();
        w.for_instance("track.synth", &entry).unwrap().reopen = true;
        let text = w.to_json().unwrap();
        let mut restored = Workspace::from_json(&text).unwrap();
        assert!(restored.windows["track.synth"].reopen);
        let other = RackEntry {
            path: entry.path,
            plugin_id: "different".into(),
        };
        assert!(!restored.for_instance("track.synth", &other).unwrap().reopen);
        restored.windows.get_mut("track.synth").unwrap().scale = 10.;
        assert!(restored.to_json().is_err());
        assert!(Workspace::from_json(&" ".repeat(262145)).is_err());
    }
}
