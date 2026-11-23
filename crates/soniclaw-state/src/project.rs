use crate::{Preset, StateError};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

/// Control-thread document. PCM and delay/voice history are external resources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tuning: Option<soniclaw_tuning::Tuning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<soniclaw_transport::Timeline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<soniclaw_automation::Document>,
    pub schema_version: u32,
    pub product_id: String,
    pub name: String,
    pub parameters: BTreeMap<String, f32>,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub zones: Vec<SampleZone>,
    #[serde(default)]
    pub nodes: Vec<RackNode>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub outputs: BTreeMap<String, f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulation: Option<soniclaw_dsp::modulation::ModulationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<soniclaw_midi::performance::PerformanceConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layout: BTreeMap<String, [f32; 2]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub path: String,
    pub root_note: u8,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleZone {
    #[serde(default, skip_serializing_if = "is_false")]
    pub release_trigger: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_fade_in: Option<[u8; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_fade_out: Option<[u8; 2]>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub velocity_power: bool,
    pub asset: String,
    pub keys: [u8; 2],
    pub velocities: [u8; 2],
    pub group: u16,
    pub gain: f32,
    pub one_shot: bool,
    pub loop_frames: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start_frame: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_frame: Option<usize>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub loop_crossfade: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessorKind {
    Custom,
    PolyModSynth,
    Granular,
    Convolution,
    Multiband,
    DynamicEq,
    SpectralFreeze,
    Robotisation,
    PitchShift,
    FrequencyShift,
    Gain,
    LowPass,
    Eq,
    Compressor,
    Gate,
    Chorus,
    Reverb,
    Limiter,
    Drive,
    Delay,
    Sampler,
    DiskSampler,
    Wavetable,
    StereoWidth,
    StateVariable,
    Ladder,
    CrossoverLow,
    CrossoverMid,
    CrossoverHigh,
    RmsCompressor,
    LookaheadLimiter,
    TransientShaper,
    SpectralGate,
    FdnReverb,
    Phaser,
    Flanger,
    Tremolo,
    AutoPan,
    MultiTapDelay,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomProcessorState>,
    pub id: String,
    pub kind: ProcessorKind,
    /// Parameter keys are local to this node. Global IDs are `node_id.key`.
    pub settings: BTreeMap<String, f32>,
    pub input_gain: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_modulation: Option<soniclaw_dsp::voice_mod::VoiceModulationConfig>,
}
/// Versioned configuration for a compiled-in registered processor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProcessorState {
    pub processor_id: String,
    pub version: u32,
    pub data: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub from: String,
    pub to: String,
    pub gain: f32,
}
fn is_false(v: &bool) -> bool {
    !*v
}
fn is_zero(n: &usize) -> bool {
    *n == 0
}
impl SampleZone {
    pub fn new(asset: impl Into<String>) -> Self {
        Self {
            release_trigger: false,
            velocity_fade_in: None,
            velocity_fade_out: None,
            velocity_power: false,
            asset: asset.into(),
            keys: [48, 72],
            velocities: [1, 127],
            group: 0,
            gain: 1.0,
            one_shot: true,
            loop_frames: None,
            start_frame: 0,
            end_frame: None,
            loop_crossfade: 0,
        }
    }
}
impl ProjectState {
    pub fn empty(product: &str, name: &str) -> Self {
        Self {
            schema_version: 2,
            tuning: None,
            automation: None,
            timeline: None,
            product_id: product.into(),
            name: name.into(),
            parameters: BTreeMap::new(),
            assets: vec![],
            zones: vec![],
            nodes: vec![],
            connections: vec![],
            outputs: BTreeMap::new(),
            modulation: None,
            midi: None,
            layout: BTreeMap::new(),
        }
    }
    /// Migrates v1 parameter-only presets to a v2 document with no structural edits.
    pub fn from_json(json: &str) -> Result<Self, StateError> {
        if json.len() > 4 * 1024 * 1024 {
            return Err("project exceeds 4 MiB".into());
        }
        let value: serde_json::Value = serde_json::from_str(json)?;
        let result = match value.get("schema_version").and_then(|x| x.as_u64()) {
            Some(1) => {
                let p: Preset = serde_json::from_value(value)?;
                let mut state = Self::empty(&p.product_id, &p.name);
                state.parameters = p.parameters;
                state
            }
            Some(2) => serde_json::from_value(value)?,
            _ => return Err("unsupported project schema".into()),
        };
        result.validate(&result.product_id)?;
        Ok(result)
    }
    pub fn to_json(&self) -> Result<String, StateError> {
        self.validate(&self.product_id)?;
        let json = serde_json::to_string_pretty(self)?;
        if json.len() > 4 * 1024 * 1024 {
            return Err("project exceeds 4 MiB".into());
        }
        Ok(json)
    }
    pub fn validate(&self, product: &str) -> Result<(), StateError> {
        if let Some(t) = &self.tuning {
            t.validate()?;
        }
        if let Some(t) = &self.timeline {
            soniclaw_transport::TempoMap::compile(t)?;
        }
        if let Some(a) = &self.automation {
            a.validate()?;
        }
        if let Some(m) = &self.midi {
            m.validate()?;
        }
        if self.layout.len() > 256
            || self.layout.iter().any(|(id, p)| {
                !self.nodes.iter().any(|n| &n.id == id)
                    || p.iter()
                        .any(|v| !v.is_finite() || !(0.0..=10000.0).contains(v))
            })
        {
            return Err("invalid graph layout".into());
        }
        for n in &self.nodes {
            match (&n.custom, n.kind) {
                (Some(c), ProcessorKind::Custom)
                    if !c.processor_id.is_empty()
                        && c.processor_id.len() <= 128
                        && c.version > 0
                        && c.version <= 65535
                        && serde_json::to_vec(&c.data)?.len() <= 65536 => {}
                (None, kind) if kind != ProcessorKind::Custom => {}
                _ => return Err("invalid custom processor state".into()),
            }
            if let Some(c) = &n.voice_modulation {
                if n.kind != ProcessorKind::PolyModSynth {
                    return Err("voice modulation requires PolyModSynth".into());
                }
                c.validate()?;
            }
        }

        if self.schema_version != 2
            || self.product_id.is_empty()
            || self.product_id != product
            || self.nodes.len() > 256
            || self.assets.len() > 1024
            || self.zones.len() > 1024
            || self.connections.len() > 4096
            || self.parameters.len() > 4096
        {
            return Err("invalid project identity or limits".into());
        }
        if self.parameters.values().any(|v| !v.is_finite()) {
            return Err("non-finite parameter".into());
        }
        let mut assets = BTreeSet::new();
        for a in &self.assets {
            if a.id.is_empty()
                || !assets.insert(&a.id)
                || a.root_note > 127
                || !safe_relative(&a.path)
            {
                return Err("invalid/duplicate asset or unsafe relative path".into());
            }
        }
        for z in &self.zones {
            if (z.release_trigger && (!z.one_shot || z.loop_frames.is_some()))
                || [z.velocity_fade_in, z.velocity_fade_out]
                    .iter()
                    .flatten()
                    .any(|[a, b]| a >= b || *b > 127)
                || !assets.contains(&z.asset)
                || z.keys[0] > z.keys[1]
                || z.keys[1] > 127
                || z.velocities[0] == 0
                || z.velocities[0] > z.velocities[1]
                || z.velocities[1] > 127
                || z.group > 255
                || !z.gain.is_finite()
                || !(0.0..=8.0).contains(&z.gain)
                || z.end_frame.is_some_and(|b| b <= z.start_frame)
                || z.loop_crossfade > 65536
                || (z.loop_crossfade > 0 && z.loop_frames.is_none())
                || z.loop_frames.is_some_and(|[a, b]| {
                    a < z.start_frame
                        || a >= b
                        || z.one_shot
                        || z.end_frame.is_some_and(|end| b > end)
                        || z.loop_crossfade > (b - a) / 2
                })
            {
                return Err("invalid sample zone".into());
            }
        }
        if let Some(m) = &self.modulation {
            m.validate()?;
        }
        let mut ids = BTreeMap::new();
        for (index, n) in self.nodes.iter().enumerate() {
            if n.asset.as_ref().is_some_and(|a| !assets.contains(a)) {
                return Err("unknown node asset".into());
            }
            if n.id.is_empty()
                || n.id.contains('.')
                || ids.insert(&n.id, index).is_some()
                || !n.input_gain.is_finite()
                || n.settings.len() > 32
                || n.settings.values().any(|v| !v.is_finite())
            {
                return Err("invalid rack node".into());
            }
        }
        let mut degree = vec![0usize; self.nodes.len()];
        let mut pairs = BTreeSet::new();
        for c in &self.connections {
            let (Some(&a), Some(&b)) = (ids.get(&c.from), ids.get(&c.to)) else {
                return Err("unknown connection node".into());
            };
            if a == b || !c.gain.is_finite() || !pairs.insert((a, b)) {
                return Err("invalid/duplicate connection".into());
            }
            degree[b] += 1;
        }
        let mut ready: Vec<_> = degree
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut visited = 0;
        while let Some(i) = ready.pop() {
            visited += 1;
            for c in &self.connections {
                if ids[&c.from] == i {
                    let to = ids[&c.to];
                    degree[to] -= 1;
                    if degree[to] == 0 {
                        ready.push(to);
                    }
                }
            }
        }
        if visited != self.nodes.len()
            || self
                .outputs
                .iter()
                .any(|(id, g)| !ids.contains_key(id) || !g.is_finite())
            || (!self.nodes.is_empty() && self.outputs.is_empty())
        {
            return Err("cyclic graph or missing/invalid output".into());
        }
        Ok(())
    }
    /// Resolves and checks symlinks on the control thread. Assets must remain in root.
    pub fn resolve_asset(&self, root: &Path, asset: &Asset) -> Result<PathBuf, StateError> {
        if !safe_relative(&asset.path) {
            return Err("unsafe asset path".into());
        }
        let root = root.canonicalize()?;
        let file = root.join(&asset.path).canonicalize()?;
        if !file.starts_with(root) {
            return Err("asset escapes project directory".into());
        }
        Ok(file)
    }
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Bounded snapshot undo/redo for document edits, including banks and topology.
/// Never use on the audio thread. Failed edits leave both history and state intact.
#[derive(Clone)]
pub struct ProjectHistory {
    current: ProjectState,
    undo: Vec<(String, ProjectState)>,
    redo: Vec<(String, ProjectState)>,
    capacity: usize,
}
impl ProjectHistory {
    pub fn new(current: ProjectState, capacity: usize) -> Result<Self, StateError> {
        current.validate(&current.product_id)?;
        if capacity == 0 || capacity > 1024 {
            return Err("invalid history capacity".into());
        }
        Ok(Self {
            current,
            undo: vec![],
            redo: vec![],
            capacity,
        })
    }
    pub fn current(&self) -> &ProjectState {
        &self.current
    }
    pub fn edit(
        &mut self,
        label: &str,
        edit: impl FnOnce(&mut ProjectState),
    ) -> Result<(), StateError> {
        let mut next = self.current.clone();
        edit(&mut next);
        next.validate(&self.current.product_id)?;
        if next == self.current {
            return Ok(());
        }
        if self.undo.len() == self.capacity {
            self.undo.remove(0);
        }
        self.undo
            .push((label.into(), std::mem::replace(&mut self.current, next)));
        self.redo.clear();
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        if let Some((label, previous)) = self.undo.pop() {
            self.redo
                .push((label, std::mem::replace(&mut self.current, previous)));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some((label, next)) = self.redo.pop() {
            self.undo
                .push((label, std::mem::replace(&mut self.current, next)));
            true
        } else {
            false
        }
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|x| x.0.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|x| x.0.as_str())
    }
}

impl RackNode {
    pub fn new(id: impl Into<String>, kind: ProcessorKind) -> Self {
        Self {
            custom: None,
            id: id.into(),
            kind,
            settings: BTreeMap::new(),
            input_gain: 0.0,
            asset: None,
            voice_modulation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_history_and_transactional_failure() {
        let mut p = ProjectState::from_json(
            r#"{"schema_version":1,"product_id":"demo","name":"Old","parameters":{"gain":0.5}}"#,
        )
        .unwrap();
        assert_eq!(p.schema_version, 2);
        p.assets.push(Asset {
            id: "kick".into(),
            path: "samples/kick.wav".into(),
            root_note: 36,
        });
        let mut h = ProjectHistory::new(p, 2).unwrap();
        h.edit("Change", |p| {
            p.parameters
                .insert("gain".into(), 0.7)
                .map(|_| ())
                .unwrap_or(())
        })
        .unwrap();
        let saved = h.current().to_json().unwrap();
        assert!(h.undo());
        assert_eq!(h.current().parameters["gain"], 0.5);
        assert!(h.redo());
        assert_eq!(h.current().to_json().unwrap(), saved);
        assert!(h
            .edit("Bad", |p| p.assets[0].path = "../escape.wav".into())
            .is_err());
        assert_eq!(h.current().to_json().unwrap(), saved);
        assert!(ProjectState::from_json(
            &saved.replace("schema_version\": 2", "schema_version\": 99")
        )
        .is_err());
    }
    #[test]
    fn rejects_cycles() {
        let mut p = ProjectState::empty("test", "test");
        for id in ["a", "b"] {
            p.nodes.push(RackNode {
                custom: None,
                voice_modulation: None,
                asset: None,
                id: id.into(),
                kind: ProcessorKind::Gain,
                settings: BTreeMap::new(),
                input_gain: 0.0,
            });
        }
        p.outputs.insert("b".into(), 1.0);
        p.connections = vec![
            Connection {
                from: "a".into(),
                to: "b".into(),
                gain: 1.0,
            },
            Connection {
                from: "b".into(),
                to: "a".into(),
                gain: 1.0,
            },
        ];
        assert!(p.validate("test").is_err());
    }
}
