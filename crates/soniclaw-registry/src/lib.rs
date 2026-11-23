//! Register compiled Rust DSP on a control thread; prepared callbacks do no registry lookup.
pub use serde_json::Value;
use soniclaw_core::*;
use soniclaw_state::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

#[derive(Clone, Copy, Debug)]
pub struct ProcessorMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub version: u32,
    pub instrument: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ParameterSpec {
    pub key: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
}
pub struct BuildContext<'a> {
    pub node: &'a RackNode,
    pub project: &'a ProjectState,
    pub asset_root: &'a Path,
    pub parameters: &'a [Arc<Parameter>],
}
type Factory =
    dyn Fn(BuildContext<'_>) -> Result<Box<dyn AudioProcessor>, StateError> + Send + Sync;
type Migration = dyn Fn(u32, &Value) -> Result<Value, StateError> + Send + Sync;
#[cfg(feature = "ui")]
type Editor = dyn Fn(&mut egui::Ui, &[Arc<Parameter>]) -> bool + Send + Sync;
pub struct Registration {
    metadata: ProcessorMetadata,
    parameters: Vec<ParameterSpec>,
    factory: Arc<Factory>,
    migrate: Option<Arc<Migration>>,
    default_state: Value,
    #[cfg(feature = "ui")]
    editor: Option<Arc<Editor>>,
}
impl Registration {
    pub fn new(
        metadata: ProcessorMetadata,
        parameters: Vec<ParameterSpec>,
        factory: impl Fn(BuildContext<'_>) -> Result<Box<dyn AudioProcessor>, StateError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            metadata,
            parameters,
            factory: Arc::new(factory),
            migrate: None,
            default_state: Value::Null,
            #[cfg(feature = "ui")]
            editor: None,
        }
    }
    pub fn with_default_state(mut self, value: Value) -> Self {
        self.default_state = value;
        self
    }
    /// Migrate an older configuration to the registration's current version.
    /// Parameter ID/range migrations remain the product's compatibility responsibility.
    pub fn with_migration(
        mut self,
        migrate: impl Fn(u32, &Value) -> Result<Value, StateError> + Send + Sync + 'static,
    ) -> Self {
        self.migrate = Some(Arc::new(migrate));
        self
    }
    #[cfg(feature = "ui")]
    pub fn with_editor(
        mut self,
        editor: impl Fn(&mut egui::Ui, &[Arc<Parameter>]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.editor = Some(Arc::new(editor));
        self
    }
}
#[derive(Default)]
pub struct ProcessorRegistry {
    entries: BTreeMap<&'static str, Registration>,
}
pub struct RegisteredProcessor {
    pub processor: Box<dyn AudioProcessor>,
    pub parameters: Vec<Arc<Parameter>>,
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
impl ProcessorRegistry {
    pub fn register(&mut self, registration: Registration) -> Result<(), StateError> {
        let m = registration.metadata;
        if self.entries.len() >= 256
            || m.id.len() > 128
            || !m.id.contains('.')
            || !m.id.split('.').all(token)
            || m.name.is_empty()
            || m.category.is_empty()
            || !(1..=65535).contains(&m.version)
            || registration.parameters.len() > 32
            || serde_json::to_vec(&registration.default_state)?.len() > 65536
        {
            return Err("invalid registration metadata or registry limit".into());
        }
        if self.entries.contains_key(m.id) {
            return Err(format!("processor already registered: {}", m.id).into());
        }
        let mut keys = BTreeSet::new();
        for p in &registration.parameters {
            if !token(p.key) || !keys.insert(p.key) || p.name.is_empty() {
                return Err("invalid/duplicate registered parameter".into());
            }
            Parameter::new(ParameterInfo {
                id: p.key,
                name: p.name,
                min: p.min,
                max: p.max,
                default: p.default,
                unit: p.unit,
            })?;
        }
        self.entries.insert(m.id, registration);
        Ok(())
    }
    pub fn entries(&self) -> impl Iterator<Item = ProcessorMetadata> + '_ {
        self.entries.values().map(|r| r.metadata)
    }
    pub fn metadata(&self, id: &str) -> Option<ProcessorMetadata> {
        self.entries.get(id).map(|r| r.metadata)
    }
    pub fn node(&self, id: impl Into<String>, processor_id: &str) -> Result<RackNode, StateError> {
        let r = self
            .entries
            .get(processor_id)
            .ok_or_else(|| format!("missing registered processor: {processor_id}"))?;
        let id = id.into();
        if !token(&id) {
            return Err("invalid custom node ID".into());
        }
        let mut node = RackNode::new(id, ProcessorKind::Custom);
        node.custom = Some(CustomProcessorState {
            processor_id: processor_id.into(),
            version: r.metadata.version,
            data: r.default_state.clone(),
        });
        Ok(node)
    }
    /// Returns a new document; failure never mutates the input document or controls.
    pub fn normalize_project(&self, state: &ProjectState) -> Result<ProjectState, StateError> {
        state.validate(&state.product_id)?;
        let mut next = state.clone();
        for n in &mut next.nodes {
            let Some(c) = &mut n.custom else { continue };
            if !token(&n.id) {
                return Err("invalid custom node ID".into());
            }
            let r = self.entries.get(c.processor_id.as_str()).ok_or_else(|| {
                format!(
                    "missing registered processor: {} (node {})",
                    c.processor_id, n.id
                )
            })?;
            if c.version > r.metadata.version {
                return Err(
                    format!("processor {} requires a newer registration", c.processor_id).into(),
                );
            }
            if c.version < r.metadata.version {
                c.data = (r
                    .migrate
                    .as_ref()
                    .ok_or("processor has no state migration")?)(
                    c.version, &c.data
                )?;
                c.version = r.metadata.version;
            }
        }
        next.validate(&next.product_id)?;
        Ok(next)
    }
    pub fn instantiate(
        &self,
        node: &RackNode,
        project: &ProjectState,
        root: &Path,
        reuse: &[Arc<Parameter>],
    ) -> Result<RegisteredProcessor, StateError> {
        let c = node
            .custom
            .as_ref()
            .ok_or("custom node has no registration ID")?;
        let r = self
            .entries
            .get(c.processor_id.as_str())
            .ok_or_else(|| format!("missing registered processor: {}", c.processor_id))?;
        if node.kind != ProcessorKind::Custom
            || !token(&node.id)
            || c.version != r.metadata.version
            || serde_json::to_vec(&c.data)?.len() > 65536
        {
            return Err("custom node requires normalization".into());
        }
        if node
            .settings
            .keys()
            .any(|key| !r.parameters.iter().any(|p| p.key == key))
        {
            return Err("unknown registered setting".into());
        }
        let mut parameters = Vec::new();
        for p in &r.parameters {
            let id = format!("{}.{}", node.id, p.key);
            if let Some(v) = node.settings.get(p.key) {
                if !v.is_finite() || !(p.min..=p.max).contains(v) {
                    return Err("registered setting out of range".into());
                }
            }
            if let Some(old) = reuse.iter().find(|old| old.info().id == id) {
                let i = old.info();
                if i.min != p.min
                    || i.max != p.max
                    || i.default != p.default
                    || i.name != p.name
                    || i.unit != p.unit
                {
                    return Err("registered host parameter metadata changed".into());
                }
                parameters.push(old.clone());
            } else {
                if !reuse.is_empty() {
                    return Err("registered host parameter identity changed".into());
                }
                parameters.push(Arc::new(Parameter::new(ParameterInfo {
                    id: &id,
                    name: p.name,
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    unit: p.unit,
                })?));
            }
        }
        let processor = (r.factory)(BuildContext {
            node,
            project,
            asset_root: root,
            parameters: &parameters,
        })?;
        Ok(RegisteredProcessor {
            processor,
            parameters,
        })
    }
    /// Some(commit) when a registration supplies an editor; None requests generic controls.
    #[cfg(feature = "ui")]
    pub fn show_editor(
        &self,
        processor_id: &str,
        ui: &mut egui::Ui,
        parameters: &[Arc<Parameter>],
    ) -> Option<bool> {
        self.entries
            .get(processor_id)
            .and_then(|r| r.editor.as_ref())
            .map(|editor| editor(ui, parameters))
    }
}
