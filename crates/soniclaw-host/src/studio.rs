//! Control-thread native ownership for Studio graphs. Explicit trust is required for each module.
pub mod automation;
#[cfg(feature = "vst3")]
pub mod bus_adapter;
pub mod isolation_adapter;
pub const READ_ONLY: u32 = clap_sys::ext::params::CLAP_PARAM_IS_READONLY;
pub mod registry;
use crate::{
    formats::{PluginFormat, Product, TrustedLoader},
    live::{LiveRack, RackEntry},
    *,
};
use soniclaw_studio::{
    self as studio,
    native_state::{Blob, Identity, States},
    Built, Descriptor,
};
use std::{collections::BTreeMap, sync::Arc};
pub enum Backend {
    #[cfg(feature = "isolation")]
    IsolatedVst3(Box<soniclaw_vst3::isolation::Isolated>),
    Clap(LiveRack),
    #[cfg(feature = "vst3")]
    Vst3(soniclaw_vst3::Instance),
}
pub struct Owner {
    pub last_good: Option<Blob>,
    pub fault: Option<String>,
    pub id: String,
    pub entry: RackEntry,
    pub restore_window: bool,
    pub format: PluginFormat,
    pub backend: Backend,
    parameters: Vec<ParameterInfo>,
    latency: usize,
    version: String,
    opaque: bool,
    pub controls: Vec<automation::Control>,
    pub automation_failures: std::sync::Arc<std::sync::atomic::AtomicU64>,
    targets: BTreeMap<u32, f64>,
    editor_active: std::collections::BTreeSet<u32>,
    editor_edits: Vec<studio::native_edits::Edit>,
}
impl Owner {
    /// Validated normalized editor gesture ingress, also usable by custom native editor adapters.
    pub fn accept_editor_edit(
        &mut self,
        id: u32,
        phase: studio::native_edits::Phase,
        value: Option<f64>,
    ) -> Result<(), HostError> {
        use studio::native_edits::{Edit, Phase};
        if let Some(v) = value {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err("invalid native editor value".into());
            }
        }
        let c = self
            .controls
            .iter()
            .find(|c| c.native_id == id)
            .ok_or("unselected/read-only native editor parameter")?;
        let event = Edit {
            parameter: c.parameter.info().id.into(),
            phase,
            value: value.map(|v| v as f32),
        };
        event.validate()?;
        if self.editor_edits.len() + 3 > 256 {
            return Err("native editor gesture queue full; stop and service gestures".into());
        }
        if phase == Phase::End && !self.editor_active.contains(&id) {
            return Ok(());
        }
        let single = phase == Phase::Value && !self.editor_active.contains(&id);
        if single {
            self.editor_edits.push(Edit {
                parameter: event.parameter.clone(),
                phase: Phase::Begin,
                value: None,
            });
        }
        if phase == Phase::Begin && !self.editor_active.insert(id) {
            return Ok(());
        }
        if let Some(value) = value {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("invalid native editor normalized value".into());
            }
            c.parameter.set_normalized(value as f32)?;
            self.targets.remove(&id);
            if let Some(p) = self.parameters.iter_mut().find(|p| p.id == id) {
                p.value = c.min + value * (c.max - c.min);
            }
        }
        if phase == Phase::End {
            self.editor_active.remove(&id);
        }
        self.editor_edits.push(event.clone());
        if single {
            self.editor_edits.push(Edit {
                phase: Phase::End,
                value: None,
                ..event
            });
        }
        Ok(())
    }
    pub fn take_editor_edits(&mut self) -> Vec<studio::native_edits::Edit> {
        std::mem::replace(&mut self.editor_edits, Vec::with_capacity(256))
    }
    pub fn finish_editor_gestures(&mut self) -> Result<(), HostError> {
        for id in self.editor_active.clone() {
            self.accept_editor_edit(id, studio::native_edits::Phase::End, None)?;
        }
        Ok(())
    }
    pub fn collect_editor_edits(&mut self) -> Result<(), HostError> {
        use studio::native_edits::Phase;
        let mut edits = Vec::new();
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => {}
            Backend::Clap(r) => {
                if r.stats().snapshot().output_drops > 0 {
                    return Err("native editor output events overflowed".into());
                }
                for e in r.drain_events() {
                    match e.event {
                        OutputEvent::Gesture { id, begin, .. } => edits.push((
                            id,
                            if begin { Phase::Begin } else { Phase::End },
                            None,
                            false,
                        )),
                        OutputEvent::Parameter(e) => {
                            if let Some(p) = self.parameters.iter_mut().find(|p| p.id == e.id) {
                                p.value = e.value;
                                self.targets.remove(&e.id);
                                if p.max > p.min {
                                    edits.push((
                                        e.id,
                                        Phase::Value,
                                        Some((e.value - p.min) / (p.max - p.min)),
                                        true,
                                    ));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            #[cfg(feature = "vst3")]
            Backend::Vst3(p) => {
                let raw = p.drain_parameter_edits()?;
                if raw.len() > 256 {
                    return Err("native editor gesture ingress exceeded 256 events".into());
                }
                for e in raw {
                    use soniclaw_vst3::ParameterEditKind;
                    edits.push((
                        e.id,
                        match e.kind {
                            ParameterEditKind::BeginGesture => Phase::Begin,
                            ParameterEditKind::ValueChange => Phase::Value,
                            ParameterEditKind::EndGesture => Phase::End,
                        },
                        e.value,
                        false,
                    ));
                }
            }
        }
        for (id, phase, value, clap_value) in edits {
            if !self.controls.iter().any(|c| c.native_id == id) {
                if phase == Phase::Value {
                    self.targets.remove(&id);
                }
                continue;
            }
            if clap_value && !self.editor_active.contains(&id) {
                continue;
            }
            self.accept_editor_edit(id, phase, value)?;
        }
        Ok(())
    }

    pub fn latency_frames(&self) -> usize {
        self.latency
    }
    pub fn parameters(&self, _: usize) -> Option<&[ParameterInfo]> {
        Some(&self.parameters)
    }
    pub fn set_parameter(&mut self, _: usize, id: u32, value: f64) -> Result<(), HostError> {
        let p = self
            .parameters
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("unknown native parameter")?;
        if !value.is_finite()
            || !(p.min..=p.max).contains(&value)
            || p.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY != 0
        {
            return Err("invalid/read-only parameter".into());
        }
        if let Some(c) = self.controls.iter().find(|c| c.native_id == id) {
            c.parameter
                .set_normalized(((value - c.min) / (c.max - c.min)) as f32)?;
        } else {
            match &mut self.backend {
                #[cfg(feature = "isolation")]
                Backend::IsolatedVst3(r) => r.set_parameter(id, value)?,
                Backend::Clap(r) => r.set_parameter(0, id, value)?,
                #[cfg(feature = "vst3")]
                Backend::Vst3(r) => r.set_parameter(id, value)?,
            };
        }
        p.value = value;
        self.targets.insert(id, value);
        Ok(())
    }
    pub fn poll(&mut self) -> Result<(), HostError> {
        if self
            .automation_failures
            .load(std::sync::atomic::Ordering::Acquire)
            > 0
        {
            self.fault = Some("native processing failed; stop and explicitly recover".into());
            return Err(self.fault.clone().unwrap().into());
        }
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(r) => {
                if let Err(e) = r.poll() {
                    self.fault = Some(e.to_string());
                    return Err(e);
                }
            }
            Backend::Clap(r) => {
                r.poll();
                if r.stopped() {
                    return Err("native restart/rescan requested; stop and rebuild playback".into());
                }
            }
            #[cfg(feature = "vst3")]
            Backend::Vst3(r) => {
                r.poll()?;
                if r.stats().process_errors > 0 {
                    return Err("native worker reported processing errors; stop and restart".into());
                }
                for p in r.parameters()? {
                    if !self.targets.contains_key(&p.id) {
                        if let Some(v) = self.parameters.iter_mut().find(|v| v.id == p.id) {
                            v.value = p.value;
                        }
                    }
                }
            }
        };
        self.collect_editor_edits()?;
        Ok(())
    }
    pub fn editor_size(&self, node: usize) -> Option<[u32; 2]> {
        match &self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => None,
            Backend::Clap(r) => r.editor_size(node),
            #[cfg(feature = "vst3")]
            Backend::Vst3(_) => None,
        }
    }
    pub fn editor_open(&self) -> bool {
        match &self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => false,
            Backend::Clap(r) => r.editor_size(0).is_some(),
            #[cfg(feature = "vst3")]
            Backend::Vst3(r) => r.editor_open(),
        }
    }
    pub fn close_editor(&mut self, node: usize) {
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => {}
            Backend::Clap(r) => r.close_editor(node),
            #[cfg(feature = "vst3")]
            Backend::Vst3(r) => r.close_editor(),
        }
    }
    pub fn open_editor(
        &mut self,
        state: &crate::editor_workspace::State,
    ) -> Result<Vec<String>, HostError> {
        state.validate()?;
        if state.module != self.entry.path || state.plugin_id != self.entry.plugin_id {
            return Err("editor identity mismatch".into());
        }
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => {
                Err("isolated editors unavailable; use host controls".into())
            }
            Backend::Clap(r) => state.open(r, 0, &self.id, &self.entry),
            #[cfg(feature = "vst3")]
            Backend::Vst3(r) => {
                r.open_editor()?;
                Ok(vec![])
            }
        }
    }
    pub fn drain_editor_events(&mut self) -> Vec<crate::editors::EditorEvent> {
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => vec![],
            Backend::Clap(r) => r.drain_editor_events(),
            #[cfg(feature = "vst3")]
            Backend::Vst3(_) => vec![],
        }
    }
    pub fn focus_editor(&mut self, node: usize) -> Result<(), HostError> {
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => Err("isolated editors unavailable".into()),
            Backend::Clap(r) => r.focus_editor(node),
            #[cfg(feature = "vst3")]
            Backend::Vst3(_) => Err("use the native VST3 window controls".into()),
        }
    }
    pub fn resize_editor(&mut self, node: usize, size: [u32; 2]) -> Result<(), HostError> {
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => Err("isolated editors unavailable".into()),
            Backend::Clap(r) => r.resize_editor(node, size),
            #[cfg(feature = "vst3")]
            Backend::Vst3(_) => Err("use the native VST3 window controls".into()),
        }
    }
    pub fn scale_editor(&mut self, node: usize, scale: f64) -> Result<(), HostError> {
        match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(_) => Err("isolated editors unavailable".into()),
            Backend::Clap(r) => r.scale_editor(node, scale),
            #[cfg(feature = "vst3")]
            Backend::Vst3(_) => Err("use the native VST3 window controls".into()),
        }
    }
    pub fn isolated_pid(&self) -> Option<u32> {
        match &self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(r) => r.helper_pid(),
            _ => None,
        }
    }
    #[cfg(feature = "isolation")]
    fn recovery_snapshot(&self) -> Result<Blob, HostError> {
        let old = self.last_good.as_ref().ok_or("no last good checkpoint")?;
        let mut params = old.metadata().parameters.clone();
        params.extend(&self.targets);
        for c in &self.controls {
            params.insert(c.native_id, c.native_value());
        }
        Blob::new(
            old.metadata().identity.clone(),
            params,
            old.metadata().opaque.then(|| old.bytes().to_vec()),
        )
    }
    pub fn snapshot(&mut self) -> Result<Blob, HostError> {
        self.collect_editor_edits()?;
        self.close_editor(0);
        #[cfg(feature = "isolation")]
        if let Backend::IsolatedVst3(r) = &mut self.backend {
            if r.status().audio.process_errors > 0 || r.offline_errors() > 0 || self.fault.is_some()
            {
                self.fault.get_or_insert_with(|| {
                    "helper failed; using last good checkpoint plus host controls".into()
                });
                return self.recovery_snapshot();
            }
        }
        let (mut params, bytes) = match &mut self.backend {
            #[cfg(feature = "isolation")]
            Backend::IsolatedVst3(r) => {
                let result = (|| -> Result<_, HostError> {
                    r.stop()?;
                    let params = r
                        .parameters()?
                        .iter()
                        .filter(|p| !p.is_read_only)
                        .map(|p| (p.id, p.value))
                        .collect();
                    Ok((
                        params,
                        if self.opaque {
                            Some(r.save_state()?.bytes)
                        } else {
                            None
                        },
                    ))
                })();
                match result {
                    Ok(value) => value,
                    Err(error) => {
                        if r.poll().is_err() {
                            self.fault=Some(format!("helper failed during snapshot: {error}; using last good checkpoint"));
                            return self.recovery_snapshot();
                        }
                        return Err(error);
                    }
                }
            }
            Backend::Clap(r) => {
                r.stop();
                let p = &mut r.instances[0];
                p.deactivate();
                p.inspect()?;
                let params = p
                    .parameters()
                    .iter()
                    .filter(|p| p.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY == 0)
                    .map(|p| (p.id, p.value))
                    .collect::<BTreeMap<_, _>>();
                let ext = unsafe {
                    (*p.plugin).get_extension.expect("checked")(
                        p.plugin,
                        clap_sys::ext::state::CLAP_EXT_STATE.as_ptr(),
                    )
                };
                (
                    params,
                    if !self.opaque || ext.is_null() {
                        None
                    } else {
                        Some(p.save_state()?)
                    },
                )
            }
            #[cfg(feature = "vst3")]
            Backend::Vst3(r) => {
                r.stop()?;
                let bytes = if self.opaque {
                    Some(r.save_state()?.bytes)
                } else {
                    None
                };
                let params = r
                    .parameters()?
                    .iter()
                    .filter(|p| !p.is_read_only)
                    .map(|p| (p.id, p.value))
                    .collect();
                (params, bytes)
            }
        };
        params.extend(&self.targets);
        for c in &self.controls {
            params.insert(c.native_id, c.native_value());
        }
        Blob::new(
            Identity {
                format: if self.format == PluginFormat::Clap {
                    "clap"
                } else {
                    "vst3"
                }
                .into(),
                module: self.entry.path.clone(),
                product: self.entry.plugin_id.clone(),
                version: self.version.clone(),
            },
            params,
            bytes,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    Offline,
}
pub struct NativeFactory<'a> {
    pub spec: ProcessSpec,
    pub helpers: &'a isolation_adapter::TrustedHelpers,
    pub bus_graph: &'a mut studio::native_buses::Registry,
    pub mode: Mode,
    pub timeline: &'a soniclaw_transport::Timeline,
    pub assets: &'a soniclaw_clips::Assets,
    pub states: &'a States,
    pub loader: &'a TrustedLoader,
    pub owners: &'a mut Vec<Owner>,
    pub last_error: Option<String>,
}
impl NativeFactory<'_> {
    #[cfg(feature = "vst3")]
    fn create_bus_instance(
        &mut self,
        d: &Descriptor,
        product: &Product,
        state: Option<Blob>,
        request: studio::native_buses::Request,
    ) -> Result<Built, HostError> {
        let spec = ProcessSpec {
            max_block_frames: self.spec.max_block_frames.max(soniclaw_vst3::BLOCK),
            ..self.spec
        };
        let mut bus = self.loader.load_vst3_buses(product, spec)?;
        for (i, b) in request.layout.inputs.iter().enumerate() {
            bus.set_input_role(i, b.role)?;
        }
        if bus.layout() != &request.layout {
            return Err("native bus layout changed; inspect and rebuild routing".into());
        }
        let p = bus.owner();
        if let Some(state) = state.filter(|s| s.matches(d)) {
            let m = state.metadata();
            if m.opaque {
                p.restore_state(&soniclaw_vst3::State {
                    schema: 1,
                    path: product.path.clone(),
                    class_id: product.id.clone(),
                    version: m.identity.version.clone(),
                    bytes: state.bytes().to_vec(),
                    sha256: m.sha256.clone(),
                })?;
            }
            for (id, value) in &m.parameters {
                p.set_parameter(*id, *value)?;
            }
        }
        let parameters = p
            .parameters()?
            .into_iter()
            .map(|p| ParameterInfo {
                id: p.id,
                name: p.name,
                min: 0.,
                max: 1.,
                default: p.default,
                value: p.value,
                flags: if p.is_read_only { READ_ONLY } else { 0 },
            })
            .collect::<Vec<_>>();
        let version = p.info()?.version;
        let driver = if self.mode == Mode::Offline {
            bus_adapter::Driver::Offline(Box::new(bus.start_offline(
                soniclaw_vst3::musical::Config {
                    timeline: self.timeline.clone(),
                    recording: false,
                },
            )?))
        } else {
            bus_adapter::Driver::Live(Box::new(bus.start_buffered()?))
        };
        let processor =
            bus_adapter::Adapter::new(driver, request, self.bus_graph, &d.id, self.spec)?;
        let latency = processor.latency_frames();
        let controls = automation::controls(d, &parameters)?;
        let handles = controls.iter().map(|c| c.parameter.clone()).collect();
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        self.owners.push(Owner {
            last_good: None,
            fault: None,
            id: d.id.clone(),
            entry: RackEntry {
                path: product.path.clone(),
                plugin_id: product.id.clone(),
            },
            restore_window: true,
            format: product.format,
            backend: Backend::Vst3(bus.into_owner()),
            parameters,
            latency,
            version,
            opaque: d.state["state_mode"].as_str() != Some("parameters"),
            controls: controls.clone(),
            automation_failures: failures.clone(),
            targets: Default::default(),
            editor_active: Default::default(),
            editor_edits: Vec::with_capacity(256),
        });
        Ok(Built {
            processor: Box::new(automation::Automated::new(
                automation::NativeProcessor::Buses(Box::new(processor)),
                controls,
                failures,
            )),
            parameters: handles,
        })
    }

    pub fn create_detailed(&mut self, d: &Descriptor) -> Result<Built, HostError> {
        if d.kind == "native.return" {
            return self.bus_graph.return_processor(d).map_err(Into::into);
        }
        if !matches!(d.kind.as_str(), "clap" | "vst3") {
            return studio::Factory::create(
                &mut studio::stock::AssetsFactory {
                    assets: self.assets,
                },
                d,
            )
            .map_err(Into::into);
        }
        if d.state["bypass"].as_bool() == Some(true) {
            let latency = d.state["latency_frames"].as_u64().unwrap_or(0);
            if latency > 262144 {
                return Err("bypass latency exceeds limit".into());
            }
            if let Some(outputs) = self.bus_graph.outputs.get(&d.id) {
                for b in outputs {
                    b.latency.store(latency as usize, Ordering::Release);
                }
            }
            return Ok(Built {
                processor: Box::new(Bypass {
                    line: vec![0.; latency as usize * 2],
                    pos: 0,
                    instrument: d.state["role"].as_str() == Some("instrument"),
                }),
                parameters: automation::placeholders(d, self.states.get(&d.id))?,
            });
        }
        if d.kind == "clap" && d.state.get("buses").is_some() {
            return Err(
                "Studio native buses currently require VST3; use an explicit CLAP multibus adapter"
                    .into(),
            );
        }
        let path =
            PathBuf::from(d.state["path"].as_str().ok_or("missing native path")?).canonicalize()?;
        if let Some(expected) = d.state["module_sha256"].as_str() {
            if registry::fingerprint(&path)? != expected {
                return Err("native module changed; rescan and approve repair".into());
            }
        }
        let format = if d.kind == "clap" {
            PluginFormat::Clap
        } else {
            PluginFormat::Vst3
        };
        let id = d.state[if d.kind == "clap" {
            "plugin_id"
        } else {
            "class_id"
        }]
        .as_str()
        .ok_or("missing native product ID")?;
        let product = Product {
            format,
            path: path.clone(),
            id: id.into(),
            name: String::new(),
            vendor: String::new(),
            version: String::new(),
        };
        let state = self.states.get(&d.id);
        if d.state["native_state"].as_bool() == Some(true) && !state.is_some_and(|s| s.matches(d)) {
            return Err("missing/mismatched native snapshot".into());
        }
        if d.state.get("isolation").is_some() {
            #[cfg(feature = "isolation")]
            return self.create_isolated(d, &product, state.cloned());
            #[cfg(not(feature = "isolation"))]
            return Err("enable isolation feature".into());
        }
        let (backend, processor, parameters, latency, version): (
            Backend,
            automation::NativeProcessor,
            Vec<ParameterInfo>,
            usize,
            String,
        ) = match format {
            PluginFormat::Clap => {
                let mut p = if let Some(old) = d.state.get("session") {
                    let s = crate::session::Session::from_json(&serde_json::to_string(old)?)?;
                    if s.slots.len() != 1
                        || s.slots[0].entry.path.canonicalize()? != path
                        || s.slots[0].entry.plugin_id != id
                    {
                        return Err("legacy checkpoint identity mismatch".into());
                    }
                    self.loader
                        .resolve_clap(&s, crate::session::MissingPolicy::Fail)?
                        .instances
                        .remove(0)
                } else {
                    self.loader.load_clap(&product)?
                };
                if let Some(s) = state.filter(|s| s.matches(d)) {
                    if s.metadata().opaque {
                        p.load_state(s.bytes())?;
                    }
                    p.restore_parameters(&s.metadata().parameters)?;
                }
                let parameters = p.parameters().to_vec();
                let version = p
                    ._library
                    .plugins()
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.version.clone())
                    .unwrap_or_default();
                let latency = p.latency_frames();
                let (owner, processor) = LiveRack::start_direct(vec![p], self.spec)?;
                (
                    Backend::Clap(owner),
                    automation::NativeProcessor::Clap(processor),
                    parameters,
                    latency,
                    version,
                )
            }
            PluginFormat::Vst3 => {
                #[cfg(feature = "vst3")]
                if let Some(request) = studio::native_buses::Request::from_descriptor(d)? {
                    return self.create_bus_instance(d, &product, state.cloned(), request);
                }

                #[cfg(feature = "vst3")]
                {
                    let mut p = self.loader.load_vst3(
                        &product,
                        ProcessSpec {
                            max_block_frames: self.spec.max_block_frames.max(soniclaw_vst3::BLOCK),
                            ..self.spec
                        },
                    )?;
                    if let Some(s) = state.filter(|s| s.matches(d)) {
                        let m = s.metadata();
                        if m.opaque {
                            p.restore_state(&soniclaw_vst3::State {
                                schema: 1,
                                path: path.clone(),
                                class_id: id.into(),
                                version: m.identity.version.clone(),
                                bytes: s.bytes().to_vec(),
                                sha256: m.sha256.clone(),
                            })?;
                        }
                        for (id, value) in &m.parameters {
                            p.set_parameter(*id, *value)?;
                        }
                    }
                    let parameters = p
                        .parameters()?
                        .into_iter()
                        .map(|p| ParameterInfo {
                            id: p.id,
                            name: p.name,
                            min: 0.,
                            max: 1.,
                            default: p.default,
                            value: p.value,
                            flags: if p.is_read_only {
                                clap_sys::ext::params::CLAP_PARAM_IS_READONLY
                            } else {
                                0
                            },
                        })
                        .collect();
                    let version = p.info()?.version;
                    let config = soniclaw_vst3::musical::Config {
                        timeline: self.timeline.clone(),
                        recording: false,
                    };
                    let (processor, latency) = if self.mode == Mode::Offline {
                        let processor = p.start_offline_musical(config)?;
                        let latency = processor.latency_frames();
                        (
                            automation::NativeProcessor::Offline(Box::new(processor)),
                            latency,
                        )
                    } else {
                        let latency = p.latency_frames()? + soniclaw_vst3::LATENCY;
                        (
                            automation::NativeProcessor::Vst3(Box::new(
                                p.start_buffered_musical(config)?,
                            )),
                            latency,
                        )
                    };
                    (Backend::Vst3(p), processor, parameters, latency, version)
                }
                #[cfg(not(feature = "vst3"))]
                {
                    return Err("enable VST3 hosting".into());
                }
            }
        };
        let controls = automation::controls(d, &parameters)?;
        let handles = controls.iter().map(|c| c.parameter.clone()).collect();
        let automation_failures = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let processor = Box::new(automation::Automated::new(
            processor,
            controls.clone(),
            automation_failures.clone(),
        ));
        self.owners.push(Owner {
            last_good: None,
            fault: None,
            controls,
            automation_failures,
            id: d.id.clone(),
            entry: RackEntry {
                path,
                plugin_id: id.into(),
            },
            restore_window: true,
            format,
            backend,
            parameters,
            latency,
            version,
            opaque: d.state["state_mode"].as_str() != Some("parameters"),
            targets: BTreeMap::new(),
            editor_active: Default::default(),
            editor_edits: Vec::with_capacity(256),
        });
        Ok(Built {
            processor,
            parameters: handles,
        })
    }
}
impl studio::Factory for NativeFactory<'_> {
    fn prepare_document(&mut self, d: &studio::Document) -> Result<(), Error> {
        self.bus_graph.prepare(d)
    }
    fn track_tap(&self, id: &str) -> Option<Arc<studio::native_buses::Frames>> {
        self.bus_graph.tracks.get(id).cloned()
    }

    fn create(&mut self, d: &Descriptor) -> Result<Built, Error> {
        self.create_detailed(d).map_err(|e| {
            self.last_error = Some(e.to_string());
            Error::InvalidParameter
        })
    }
}
struct Bypass {
    line: Vec<f32>,
    pos: usize,
    instrument: bool,
}
impl AudioProcessor for Bypass {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.line.fill(0.);
        self.pos = 0;
    }
    fn latency_frames(&self) -> usize {
        self.line.len() / 2
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.instrument {
            b.clear();
            return;
        }
        if !self.line.is_empty() {
            for x in b.samples_mut() {
                std::mem::swap(x, &mut self.line[self.pos]);
                self.pos = (self.pos + 1) % self.line.len();
            }
        }
    }
}

/// Construct native owners on this offline worker, render once, then retire every owner on the same thread.
pub fn render_export(
    p: &studio::project::Project,
    s: studio::export::Settings,
    directory: &std::path::Path,
    loader: &TrustedLoader,
    context: &studio::jobs::Context,
) -> Result<studio::export::Report, HostError> {
    render_export_with_helpers(
        p,
        s,
        directory,
        loader,
        &isolation_adapter::TrustedHelpers::default(),
        context,
    )
}
/// Export with independently approved helpers on this offline worker.
pub fn render_export_with_helpers(
    p: &studio::project::Project,
    s: studio::export::Settings,
    directory: &Path,
    loader: &TrustedLoader,
    helpers: &isolation_adapter::TrustedHelpers,
    context: &studio::jobs::Context,
) -> Result<studio::export::Report, HostError> {
    if !s.stems.is_empty()
        && studio::native_state::descriptors(&p.document).any(|d| {
            matches!(d.kind.as_str(), "clap" | "vst3") && d.state["bypass"].as_bool() != Some(true)
        })
    {
        return Err(
            "native export requires same-pass routed_stems instead of legacy isolated stems".into(),
        );
    }
    let mut owners = Vec::new();
    let (result, native_error) = {
        let mut factory = NativeFactory {
            helpers,
            bus_graph: &mut Default::default(),
            spec: p.document.spec(),
            mode: Mode::Offline,
            timeline: &p.document.timeline,
            assets: &p.assets,
            states: &p.native_states,
            loader,
            owners: &mut owners,
            last_error: None,
        };
        let result = studio::export::render(p, s, directory, &mut factory, context);
        let native_error = factory.last_error.take();
        (result, native_error)
    };
    let failed = owners.iter().any(|o| {
        o.automation_failures
            .load(std::sync::atomic::Ordering::Acquire)
            > 0
            || match &o.backend {
                #[cfg(feature = "isolation")]
                Backend::IsolatedVst3(p) => {
                    p.offline_errors() > 0 || p.status().audio.process_errors > 0
                }
                Backend::Clap(_) => false,
                #[cfg(feature = "vst3")]
                Backend::Vst3(p) => p.offline_errors() > 0,
            }
    });
    owners.clear();
    if failed && result.is_ok() {
        let _ = std::fs::remove_dir_all(directory);
        return Err("native offline processing failed; output removed".into());
    }
    result.map_err(|e| native_error.map_or(e, Into::into))
}
