//! Versioned mixed native/CLAP/VST3 graph sessions, reconstructed off the callback.
use crate::{
    graph::Graph,
    host::{
        self,
        formats::Product,
        session::{EditorSettings, MissingPolicy},
    },
    prelude::*,
    state::ProjectState,
    vst3,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_JSON: usize = 96 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Native {
        project: Box<ProjectState>,
        root: PathBuf,
    },
    Clap {
        slot: Box<host::session::Slot>,
    },
    Vst3 {
        product: Box<Product>,
        state: Option<Box<vst3::State>>,
        #[serde(default)]
        parameters: BTreeMap<u32, f64>,
        #[serde(default)]
        editor: EditorSettings,
        #[serde(default)]
        musical: Option<vst3::musical::Config>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub source: Source,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default)]
    pub saved_latency: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub from: String,
    pub to: String,
    pub gain: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub schema: u32,
    pub name: String,
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
    pub inputs: BTreeMap<String, f32>,
    pub outputs: BTreeMap<String, f32>,
}
impl Session {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.name.len() > 256
            || self.nodes.is_empty()
            || self.nodes.len() > 8
            || self.connections.len() > 64
            || self.outputs.is_empty()
        {
            return Err("invalid mixed session schema/size/outputs".into());
        }
        let mut ids = BTreeSet::new();
        for n in &self.nodes {
            if n.id.is_empty()
                || n.id.len() > 64
                || !n
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !ids.insert(n.id.as_str())
                || n.saved_latency > 262144
            {
                return Err("invalid/duplicate node or latency".into());
            }
            match &n.source {
                Source::Native { project, .. } => {
                    project.validate(&project.product_id)?;
                }
                Source::Clap { slot } => {
                    host::session::Session {
                        schema: 1,
                        name: self.name.clone(),
                        slots: vec![*slot.clone()],
                    }
                    .validate()
                    .map_err(|e| e.to_string())?;
                }
                Source::Vst3 {
                    product,
                    state,
                    parameters,
                    musical,
                    ..
                } => {
                    if let Some(config) = musical {
                        config.validate().map_err(|e| e.to_string())?;
                    }
                    if product.format != host::formats::PluginFormat::Vst3
                        || product.id.len() != 32
                        || !product.id.bytes().all(|b| b.is_ascii_hexdigit())
                        || parameters.len() > 4096
                        || parameters
                            .values()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                    {
                        return Err("invalid VST3 identity/parameters".into());
                    }
                    if let Some(s) = state {
                        s.validate().map_err(|e| e.to_string())?;
                        if !s.class_id.eq_ignore_ascii_case(&product.id) {
                            return Err("VST3 state class mismatch".into());
                        }
                    }
                }
            }
        }
        for (id, gain) in self.inputs.iter().chain(&self.outputs) {
            if !ids.contains(id.as_str()) || !gain.is_finite() {
                return Err("invalid input/output node or gain".into());
            }
        }
        let mut edges = BTreeSet::new();
        let mut degree: BTreeMap<&str, usize> = ids.iter().map(|id| (*id, 0)).collect();
        for e in &self.connections {
            if !ids.contains(e.from.as_str())
                || !ids.contains(e.to.as_str())
                || e.from == e.to
                || !e.gain.is_finite()
                || !edges.insert((e.from.as_str(), e.to.as_str()))
            {
                return Err("invalid/duplicate connection".into());
            }
            *degree.get_mut(e.to.as_str()).expect("exists") += 1;
        }
        let mut queue: Vec<_> = degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut cursor = 0;
        while cursor < queue.len() {
            let id = queue[cursor];
            cursor += 1;
            for e in self.connections.iter().filter(|e| e.from == id) {
                let d = degree.get_mut(e.to.as_str()).expect("exists");
                *d -= 1;
                if *d == 0 {
                    queue.push(e.to.as_str());
                }
            }
        }
        if queue.len() != self.nodes.len() {
            return Err("mixed graph contains a cycle".into());
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        let text = serde_json::to_string_pretty(self)?;
        if text.len() > MAX_JSON {
            return Err("mixed session exceeds 96 MiB".into());
        }
        Ok(text)
    }
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > MAX_JSON {
            return Err("mixed session exceeds 96 MiB".into());
        }
        let s: Self = serde_json::from_str(text)?;
        s.validate()?;
        Ok(s)
    }
    pub fn read(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(path)?
            .take((MAX_JSON + 1) as u64)
            .read_to_end(&mut bytes)?;
        Self::from_json(std::str::from_utf8(&bytes)?)
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        let text = self.to_json()?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temp = path.with_file_name(format!(".mixed-{}-{stamp}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
    /// The explicit trust capability authorizes native modules; the native factory runs on this control thread.
    pub fn build_with(
        &self,
        loader: &host::formats::TrustedLoader,
        spec: ProcessSpec,
        missing: MissingPolicy,
        mut native: impl FnMut(&ProjectState, &Path) -> Result<crate::plugin::PluginInstance>,
    ) -> Result<(Owner, Processor)> {
        self.validate()?;
        spec.validate()?;
        if spec.channels != 2 || spec.max_block_frames > 8192 {
            return Err("mixed graph requires stereo, maximum 8192 frames".into());
        }
        let gate = Arc::new(Gate::default());
        let mut owner = Owner {
            document: self.clone(),
            nodes: vec![],
            gate: gate.clone(),
            warnings: vec![],
        };
        let mut graph = Graph::default();
        let mut ids = BTreeMap::new();
        for node in &self.nodes {
            let bypass = Arc::new(AtomicBool::new(node.bypass));
            let declared = Arc::new(AtomicUsize::new(node.saved_latency));
            let loaded = (|| -> Result<(Binding, Box<dyn AudioProcessor>)> {
                match &node.source {
                    Source::Native { project, root } => {
                        let plugin = native(project, root)?;
                        let params = plugin.parameters().to_vec();
                        Ok((
                            Binding::Native(params),
                            Box::new(crate::plugin::HostedProcessor(plugin)),
                        ))
                    }
                    Source::Clap { slot } => {
                        let session = host::session::Session {
                            schema: 1,
                            name: self.name.clone(),
                            slots: vec![*slot.clone()],
                        };
                        let mut session = session;
                        session.slots[0].bypass = false;
                        let mut resolved = loader
                            .resolve_clap(&session, MissingPolicy::Fail)
                            .map_err(|e| e.to_string())?;
                        let (mut rack, callback) =
                            host::live::LiveRack::start_direct(resolved.take_instances(), spec)
                                .map_err(|e| e.to_string())?;
                        resolved.configure(&mut rack).map_err(|e| e.to_string())?;
                        Ok((Binding::Clap(Box::new(rack)), Box::new(callback)))
                    }
                    Source::Vst3 {
                        product,
                        state,
                        parameters,
                        musical,
                        ..
                    } => {
                        let mut v = loader
                            .load_vst3(
                                product,
                                ProcessSpec {
                                    max_block_frames: spec.max_block_frames.max(256),
                                    ..spec
                                },
                            )
                            .map_err(|e| e.to_string())?;
                        if let Some(s) = state {
                            v.restore_state(s).map_err(|e| e.to_string())?;
                        }
                        for (&id, &value) in parameters {
                            v.set_parameter(id, value).map_err(|e| e.to_string())?;
                        }
                        let callback = if let Some(config) = musical {
                            v.start_buffered_musical(config.clone())
                        } else {
                            v.start_buffered()
                        }
                        .map_err(|e| e.to_string())?;
                        Ok((Binding::Vst3(Box::new(v)), Box::new(callback)))
                    }
                }
            })();
            let (binding, processor) = match loaded {
                Ok(p) => p,
                Err(error) => {
                    if matches!(missing, MissingPolicy::Fail) {
                        return Err(error);
                    }
                    owner
                        .warnings
                        .push(format!("{}: {error}; silent placeholder retained", node.id));
                    (
                        Binding::Missing,
                        Box::new(Placeholder(node.saved_latency)) as Box<dyn AudioProcessor>,
                    )
                }
            };
            let id = graph.add(Bypass {
                inner: processor,
                bypass: bypass.clone(),
                dry: vec![],
                delay: vec![],
                cursor: 0,
                declared: declared.clone(),
            });
            ids.insert(node.id.clone(), id);
            owner.nodes.push((binding, bypass, declared));
        }
        for (id, &gain) in &self.inputs {
            graph.set_input(ids[id], gain)?;
        }
        for e in &self.connections {
            graph.connect(ids[&e.from], ids[&e.to], e.gain)?;
        }
        for (id, &gain) in &self.outputs {
            graph.set_output(ids[id], gain)?;
        }
        graph.prepare(spec)?;
        for (index, (binding, _, _)) in owner.nodes.iter_mut().enumerate() {
            let settings = match &self.nodes[index].source {
                Source::Clap { slot } => Some(&slot.editor),
                Source::Vst3 { editor, .. } => Some(editor),
                _ => None,
            };
            if let Some(e) = settings.filter(|e| e.open) {
                let result = match binding {
                    Binding::Clap(r) => {
                        if e.embedded {
                            r.show_embedded_editor(0, &self.nodes[index].id)
                        } else {
                            r.show_editor(0)
                        }
                    }
                    Binding::Vst3(v) => v.open_editor(),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    owner
                        .warnings
                        .push(format!("editor {}: {error}", self.nodes[index].id));
                }
            }
        }
        Ok((owner, Processor { graph, gate, spec }))
    }
    /// Native documents use the fixed built-in product identity. The loader explicitly authorizes foreign modules.
    pub fn build(
        &self,
        loader: &host::formats::TrustedLoader,
        spec: ProcessSpec,
        missing: MissingPolicy,
    ) -> Result<(Owner, Processor)> {
        self.build_with(loader, spec, missing, |project, root| {
            crate::project::create_plugin(
                crate::plugin::PluginDescriptor {
                    id: "org.soniclaw.mixed.native",
                    name: "Soniclaw mixed native rack",
                    vendor: "Soniclaw",
                    version: env!("CARGO_PKG_VERSION"),
                    is_instrument: false,
                    accepts_midi: true,
                },
                project.clone(),
                root,
            )
        })
    }
}
enum Binding {
    Native(Vec<Arc<Parameter>>),
    Clap(Box<host::live::LiveRack>),
    Vst3(Box<vst3::Instance>),
    Missing,
}
#[derive(Default)]
struct Gate {
    stop: AtomicBool,
    busy: AtomicBool,
}
struct Guard<'a>(&'a AtomicBool);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
pub struct Owner {
    document: Session,
    nodes: Vec<(Binding, Arc<AtomicBool>, Arc<AtomicUsize>)>,
    gate: Arc<Gate>,
    pub warnings: Vec<String>,
}
impl Owner {
    pub fn diagnostics(&self) -> Vec<serde_json::Value> {
        self.nodes.iter().enumerate().map(|(i,(binding,bypass,latency))|serde_json::json!({"id":self.document.nodes[i].id,"bypass":bypass.load(Ordering::Acquire),"latency":latency.load(Ordering::Acquire),"native":match binding{Binding::Clap(r)=>serde_json::to_value(r.stats().snapshot()).unwrap_or_default(),Binding::Vst3(v)=>serde_json::to_value(v.stats()).unwrap_or_default(),Binding::Native(p)=>serde_json::json!({"parameters":p.iter().map(|p|(p.info().id,p.get())).collect::<Vec<_>>()}),Binding::Missing=>serde_json::json!("missing")}})).collect()
    }
    pub fn set_bypass(&mut self, id: &str, value: bool) -> Result<()> {
        let i = self
            .document
            .nodes
            .iter()
            .position(|n| n.id == id)
            .ok_or("unknown mixed node")?;
        self.nodes[i].1.store(value, Ordering::Release);
        Ok(())
    }
    pub fn set_parameter(&mut self, node: &str, id: u32, value: f64) -> Result<()> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("normalized value must be finite and 0..1".into());
        }
        let i = self
            .document
            .nodes
            .iter()
            .position(|n| n.id == node)
            .ok_or("unknown node")?;
        match &mut self.nodes[i].0 {
            Binding::Native(params) => {
                let p = params
                    .iter()
                    .find(|p| crate::plugin::parameter_id(p.info().id) == id)
                    .ok_or("unknown parameter")?;
                p.set_normalized(value as f32)?;
                Ok(())
            }
            Binding::Clap(r) => {
                let p = r
                    .parameters(0)
                    .and_then(|ps| ps.iter().find(|p| p.id == id))
                    .ok_or("unknown parameter")?;
                r.set_parameter(0, id, p.min + value * (p.max - p.min))
                    .map_err(|e| e.to_string().into())
            }
            Binding::Vst3(v) => v.set_parameter(id, value).map_err(|e| e.to_string().into()),
            Binding::Missing => Err("node is a missing placeholder".into()),
        }
    }
    pub fn open_editor(&mut self, id: &str) -> Result<()> {
        let i = self
            .document
            .nodes
            .iter()
            .position(|n| n.id == id)
            .ok_or("unknown node")?;
        match &mut self.nodes[i].0 {
            Binding::Clap(r) => r
                .show_embedded_editor(0, id)
                .map_err(|e| e.to_string().into()),
            Binding::Vst3(v) => v.open_editor().map_err(|e| e.to_string().into()),
            _ => Err("use the native parameter UI or resolve the missing product".into()),
        }
    }
    pub fn poll(&mut self) -> Result<()> {
        for (binding, _, _) in &mut self.nodes {
            match binding {
                Binding::Clap(r) => {
                    r.poll();
                }
                Binding::Vst3(v) => v.poll().map_err(|e| e.to_string())?,
                _ => {}
            }
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.gate.stop.store(true, Ordering::Release);
        while self
            .gate
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
        let _g = Guard(&self.gate.busy);
        for (binding, _, _) in &mut self.nodes {
            match binding {
                Binding::Clap(r) => r.stop(),
                Binding::Vst3(v) => {
                    let _ = v.stop();
                }
                _ => {}
            }
        }
    }
    pub fn capture(&mut self) -> Result<Session> {
        self.stop();
        let mut next = self.document.clone();
        for (i, (binding, bypass, declared)) in self.nodes.iter_mut().enumerate() {
            next.nodes[i].bypass = bypass.load(Ordering::Acquire);
            next.nodes[i].saved_latency = declared.load(Ordering::Acquire);
            match (binding, &mut next.nodes[i].source) {
                (Binding::Native(params), Source::Native { project, .. }) => {
                    project.parameters =
                        crate::state::Preset::capture(&project.product_id, &project.name, params)?
                            .parameters;
                }
                (Binding::Clap(r), Source::Clap { slot }) => {
                    let id = slot.id.clone();
                    let session = host::session::Session::capture(
                        r,
                        std::slice::from_ref(&slot.entry),
                        "Mixed node",
                    )
                    .map_err(|e| e.to_string())?;
                    **slot = session
                        .slots
                        .into_iter()
                        .next()
                        .ok_or("missing captured slot")?;
                    slot.id = id;
                }
                (
                    Binding::Vst3(v),
                    Source::Vst3 {
                        state,
                        parameters,
                        editor,
                        ..
                    },
                ) => {
                    editor.open = v.editor_open();
                    *state = Some(Box::new(v.save_state().map_err(|e| e.to_string())?));
                    *parameters = v
                        .parameters()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .filter(|p| !p.is_read_only)
                        .map(|p| (p.id, p.value))
                        .collect();
                }
                _ => {}
            }
        }
        next.validate()?;
        self.document = next.clone();
        Ok(next)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop();
    }
}
pub struct Processor {
    graph: Graph,
    gate: Arc<Gate>,
    spec: ProcessSpec,
}
impl AudioProcessor for Processor {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        s.validate()?;
        if s.channels != self.spec.channels
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.graph.reset();
    }
    fn latency_frames(&self) -> usize {
        self.graph.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self
            .gate
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            b.clear();
            return;
        }
        let _g = Guard(&self.gate.busy);
        if self.gate.stop.load(Ordering::Acquire) {
            b.clear();
            return;
        }
        self.graph.process(b, ctx);
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        self.gate.stop.store(true, Ordering::Release);
    }
}
struct Placeholder(usize);
impl AudioProcessor for Placeholder {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        Ok(())
    }
    fn reset(&mut self) {}
    fn latency_frames(&self) -> usize {
        self.0
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        b.clear();
    }
}
struct Bypass {
    inner: Box<dyn AudioProcessor>,
    bypass: Arc<AtomicBool>,
    dry: Vec<f32>,
    delay: Vec<f32>,
    cursor: usize,
    declared: Arc<AtomicUsize>,
}
impl AudioProcessor for Bypass {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.inner.prepare(s)?;
        self.dry = vec![0.; s.max_block_frames * 2];
        let latency = self.inner.latency_frames();
        self.declared.store(latency, Ordering::Release);
        if latency > 262144 {
            return Err(Error::InvalidRouting);
        }
        self.delay = vec![0.; latency * 2];
        Ok(())
    }
    fn reset(&mut self) {
        self.inner.reset();
        self.delay.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let n = b.samples().len();
        self.dry[..n].copy_from_slice(b.samples());
        for x in &mut self.dry[..n] {
            if !self.delay.is_empty() {
                std::mem::swap(x, &mut self.delay[self.cursor]);
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
        }
        self.inner.process(b, ctx);
        if self.bypass.load(Ordering::Acquire) {
            b.samples_mut().copy_from_slice(&self.dry[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        let mut project = ProjectState::empty("org.soniclaw.mixed.native", "Native");
        let mut node = crate::state::RackNode::new("gain", crate::state::ProcessorKind::Gain);
        node.input_gain = 1.;
        node.settings.insert("gain".into(), 0.5);
        project.nodes.push(node);
        project.outputs.insert("gain".into(), 1.);
        Session {
            schema: 1,
            name: "Mixed".into(),
            nodes: vec![Node {
                id: "native".into(),
                source: Source::Native {
                    project: Box::new(project),
                    root: ".".into(),
                },
                bypass: false,
                saved_latency: 0,
            }],
            connections: vec![],
            inputs: BTreeMap::from([("native".into(), 1.)]),
            outputs: BTreeMap::from([("native".into(), 1.)]),
        }
    }
    #[test]
    fn native_graph_capture_and_owner_revocation() {
        let s = session();
        let loader = host::formats::TrustedLoader::default();
        let spec = ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 256,
        };
        let (mut owner, mut p) = s.build(&loader, spec, MissingPolicy::Fail).unwrap();
        p.prepare(spec).unwrap();
        let mut data = [1.; 512];
        let ctx = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        let info = allocation_counter::measure(|| {
            p.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx)
        });
        assert_eq!(info.count_total, 0);
        assert!(data.iter().all(|x| (*x - 0.5).abs() < 1e-6));
        owner.set_bypass("native", true).unwrap();
        let captured = owner.capture().unwrap();
        assert!(captured.nodes[0].bypass);
        let round = Session::from_json(&captured.to_json().unwrap()).unwrap();
        assert_eq!(round.nodes[0].id, "native");
        data.fill(1.);
        p.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx);
        assert!(data.iter().all(|x| *x == 0.));
    }
    #[test]
    fn cycles_duplicates_and_bad_gains_are_rejected_before_loading() {
        let mut s = session();
        s.connections.push(Connection {
            from: "native".into(),
            to: "native".into(),
            gain: 1.,
        });
        assert!(s.validate().is_err());
        let mut s = session();
        s.nodes.push(s.nodes[0].clone());
        assert!(s.validate().is_err());
        let mut s = session();
        s.outputs.insert("native".into(), f32::NAN);
        assert!(s.validate().is_err());
    }
}

impl Session {
    /// Convert the earlier serial CLAP document without executing native code.
    /// Original slot identities/state remain inside each Source::Clap payload.
    pub fn from_clap(session: host::session::Session) -> Result<Self> {
        session.validate().map_err(|e| e.to_string())?;
        if session.slots.is_empty() {
            return Err("cannot convert an empty rack".into());
        }
        let nodes: Vec<_> = session
            .slots
            .into_iter()
            .enumerate()
            .map(|(i, slot)| Node {
                id: format!("slot-{i}"),
                bypass: slot.bypass,
                saved_latency: 0,
                source: Source::Clap {
                    slot: Box::new(slot),
                },
            })
            .collect();
        let connections = nodes
            .windows(2)
            .map(|w| Connection {
                from: w[0].id.clone(),
                to: w[1].id.clone(),
                gain: 1.,
            })
            .collect();
        let first = nodes[0].id.clone();
        let last = nodes.last().expect("nonempty").id.clone();
        let result = Self {
            schema: 1,
            name: session.name,
            nodes,
            connections,
            inputs: BTreeMap::from([(first, 1.)]),
            outputs: BTreeMap::from([(last, 1.)]),
        };
        result.validate()?;
        Ok(result)
    }
}
#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn numeric_host_parameter_ids_survive_mixed_json() {
        let slot = host::session::Slot {
            id: "original slot".into(),
            entry: host::live::RackEntry {
                path: "missing.clap".into(),
                plugin_id: "test".into(),
            },
            plugin_version: "1".into(),
            bypass: true,
            editor: Default::default(),
            parameters: BTreeMap::from([(9, 0.75)]),
            state_hex: None,
            state_sha256: None,
        };
        let session = Session::from_clap(host::session::Session {
            schema: 1,
            name: "Old".into(),
            slots: vec![slot],
        })
        .unwrap();
        let round = Session::from_json(&session.to_json().unwrap()).unwrap();
        assert!(round.nodes[0].bypass);
        let Source::Clap { slot } = &round.nodes[0].source else {
            panic!("wrong format")
        };
        assert_eq!(slot.parameters[&9], 0.75);
        assert_eq!(slot.id, "original slot");
    }
}
