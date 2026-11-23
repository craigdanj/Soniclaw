//! Versioned control-thread hosted rack sessions. Native loads remain explicit and trusted.
use super::{live::*, *};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
fn is_false(v: &bool) -> bool {
    !*v
}
const MAX_FILE: usize = 96 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorSettings {
    pub open: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub embedded: bool,
    pub size: Option<[u32; 2]>,
    pub scale: f64,
}
impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            open: false,
            embedded: false,
            size: None,
            scale: 1.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub id: String,
    pub entry: RackEntry,
    pub plugin_version: String,
    pub bypass: bool,
    pub editor: EditorSettings,
    pub parameters: BTreeMap<u32, f64>,
    pub state_hex: Option<String>,
    pub state_sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub schema: u32,
    pub name: String,
    pub slots: Vec<Slot>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Missing {
    pub slot: usize,
    pub reason: String,
}
#[derive(Clone, Copy)]
pub enum MissingPolicy {
    Fail,
    Placeholder,
}
pub struct ResolvedSession {
    pub instances: Vec<ClapInstance>,
    pub slot_indices: Vec<usize>,
    pub missing: Vec<Missing>,
    pub warnings: Vec<String>,
    bypass: u32,
}
fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|x| format!("{x:02x}")).collect()
}
fn decode(hex: &str) -> Result<Vec<u8>, HostError> {
    if hex.len() > STATE_LIMIT * 2 || !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return Err("invalid/oversized state encoding".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(Into::into))
        .collect()
}
impl Session {
    pub fn validate(&self) -> Result<(), HostError> {
        if self.schema != 1
            || self.name.len() > 256
            || self.slots.is_empty()
            || self.slots.len() > MAX_NODES
        {
            return Err("invalid session schema/name/slot count".into());
        }
        let mut ids = BTreeSet::new();
        for slot in &self.slots {
            if slot.id.is_empty()
                || slot.id.len() > 256
                || !ids.insert(&slot.id)
                || slot.entry.plugin_id.is_empty()
                || slot.entry.plugin_id.len() > 512
                || slot.entry.path.as_os_str().is_empty()
                || slot.parameters.len() > 4096
                || slot.parameters.values().any(|x| !x.is_finite())
                || !slot.editor.scale.is_finite()
                || !(0.5..=4.0).contains(&slot.editor.scale)
                || slot
                    .editor
                    .size
                    .is_some_and(|s| s.iter().any(|n| !(64..=8192).contains(n)))
            {
                return Err("invalid session slot/settings".into());
            }
            match (&slot.state_hex, &slot.state_sha256) {
                (Some(hex), Some(hash)) => {
                    let bytes = decode(hex)?;
                    if encode(&Sha256::digest(&bytes)) != *hash {
                        return Err("opaque state checksum mismatch".into());
                    }
                }
                (None, None) => {}
                _ => return Err("state/checksum must be paired".into()),
            }
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, HostError> {
        self.validate()?;
        let s = serde_json::to_string_pretty(self)?;
        if s.len() > MAX_FILE {
            return Err("session exceeds 96 MiB".into());
        }
        Ok(s)
    }
    pub fn from_json(s: &str) -> Result<Self, HostError> {
        if s.len() > MAX_FILE {
            return Err("session exceeds 96 MiB".into());
        }
        let d: Self = serde_json::from_str(s)?;
        d.validate()?;
        Ok(d)
    }
    pub fn read(path: &Path) -> Result<Self, HostError> {
        use std::io::Read;
        let mut s = String::new();
        fs::File::open(path)?
            .take((MAX_FILE + 1) as u64)
            .read_to_string(&mut s)?;
        Self::from_json(&s)
    }
    /// Atomic same-directory replacement, with complete validation before file creation.
    pub fn write(&self, path: &Path) -> Result<(), HostError> {
        use std::io::Write;
        let data = self.to_json()?;
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tmp = path.with_extension(format!(
            "session-tmp-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(data.as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, path)?;
            Ok::<_, HostError>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        result
    }
    /// # Safety
    /// All configured native paths must be explicitly trusted ABI-conforming CLAP code.
    pub unsafe fn resolve(&self, policy: MissingPolicy) -> Result<ResolvedSession, HostError> {
        self.validate()?;
        let mut libraries = BTreeMap::new();
        let mut instances = vec![];
        let mut indices = vec![];
        let mut missing = vec![];
        let mut warnings = vec![];
        let mut bypass = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            let path = match slot.entry.path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    if matches!(policy, MissingPolicy::Fail) {
                        return Err(e.into());
                    }
                    missing.push(Missing {
                        slot: index,
                        reason: e.to_string(),
                    });
                    continue;
                }
            };
            if !libraries.contains_key(&path) {
                libraries.insert(path.clone(), unsafe { ClapLibrary::load(&path) }?);
            }
            let lib = &libraries[&path];
            let Some(info) = lib.plugins().iter().find(|p| p.id == slot.entry.plugin_id) else {
                if matches!(policy, MissingPolicy::Fail) {
                    return Err("session product unavailable".into());
                }
                missing.push(Missing {
                    slot: index,
                    reason: "product ID unavailable".into(),
                });
                continue;
            };
            if info.version != slot.plugin_version {
                warnings.push(format!(
                    "{}: saved {}, installed {}",
                    slot.id, slot.plugin_version, info.version
                ));
            }
            let mut instance = lib.instantiate(&slot.entry.plugin_id)?;
            if let Some(hex) = &slot.state_hex {
                instance.load_state(&decode(hex)?)?;
            } else {
                instance.restore_parameters(&slot.parameters)?;
            }
            if slot.bypass {
                bypass |= 1 << instances.len();
            }
            instances.push(instance);
            indices.push(index);
        }
        Ok(ResolvedSession {
            instances,
            slot_indices: indices,
            missing,
            warnings,
            bypass,
        })
    }
    /// Capture all live nodes, stopping processing first. Existing snapshots aren't modified on failure.
    pub fn capture(
        owner: &mut LiveRack,
        entries: &[RackEntry],
        name: &str,
    ) -> Result<Self, HostError> {
        if entries.len() != owner.instances.len() {
            return Err("capture entry/node mismatch".into());
        }
        let editors: Vec<_> = owner.editors.iter().map(|e| e.is_some()).collect();
        owner.stop();
        let mut slots = vec![];
        let mask = owner.shared.bypass.load(Ordering::Acquire);
        for (index, (instance, entry)) in owner.instances.iter_mut().zip(entries).enumerate() {
            instance.deactivate();
            instance.inspect()?;
            let parameters = instance
                .parameters
                .iter()
                .filter(|p| p.flags & CLAP_PARAM_IS_READONLY == 0)
                .map(|p| (p.id, p.value))
                .collect();
            let extension = unsafe {
                (*instance.plugin).get_extension.expect("checked")(
                    instance.plugin,
                    CLAP_EXT_STATE.as_ptr(),
                )
            };
            let (state_hex, state_sha256) = if extension.is_null() {
                (None, None)
            } else {
                let bytes = instance.save_state()?;
                (Some(encode(&bytes)), Some(encode(&Sha256::digest(&bytes))))
            };
            let version = instance
                ._library
                .plugins()
                .iter()
                .find(|p| p.id == entry.plugin_id)
                .map(|p| p.version.clone())
                .unwrap_or_default();
            let size = instance.context.gui_last_size.load(Ordering::Acquire);
            let mut editor = EditorSettings {
                open: editors[index],
                scale: f64::from_bits(instance.context.gui_scale.load(Ordering::Acquire)),
                embedded: instance.context.gui_last_embedded.load(Ordering::Acquire),
                size: if size == 0 {
                    None
                } else {
                    Some([(size >> 32) as u32, size as u32])
                },
            };
            if let Some(e) = &owner.editors[index] {
                editor.scale = e.scale;
                editor.embedded = e.embedded;
                unsafe {
                    if let Some(size) = (*e.extension).get_size {
                        let (mut w, mut h) = (0, 0);
                        if size(e.plugin, &mut w, &mut h)
                            && [w, h].iter().all(|n| (64..=8192).contains(n))
                        {
                            editor.size = Some([w, h]);
                        }
                    }
                }
            }
            slots.push(Slot {
                id: format!("slot-{index}"),
                entry: entry.clone(),
                plugin_version: version,
                bypass: mask & (1 << index) != 0,
                editor,
                parameters,
                state_hex,
                state_sha256,
            });
        }
        let session = Self {
            schema: 1,
            name: name.into(),
            slots,
        };
        session.validate()?;
        Ok(session)
    }
    /// Merge a capture of available nodes back into a document, preserving missing slots and stable IDs.
    pub fn merge_capture(&mut self, captured: Session, indices: &[usize]) -> Result<(), HostError> {
        if indices.len() != captured.slots.len()
            || indices.iter().any(|i| *i >= self.slots.len())
            || indices.iter().collect::<BTreeSet<_>>().len() != indices.len()
        {
            return Err("invalid capture mapping".into());
        }
        let mut candidate = self.clone();
        for (mut slot, index) in captured.slots.into_iter().zip(indices) {
            slot.id = candidate.slots[*index].id.clone();
            candidate.slots[*index] = slot;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
impl ResolvedSession {
    pub fn configure(&self, owner: &mut LiveRack) -> Result<(), HostError> {
        if owner.instances.len() != self.slot_indices.len() {
            return Err("session runtime mapping mismatch".into());
        }
        owner.shared.bypass.store(self.bypass, Ordering::Release);
        Ok(())
    }
    /// Move instances into a runtime while retaining the mapping/settings in this resolver.
    pub fn take_instances(&mut self) -> Vec<ClapInstance> {
        std::mem::take(&mut self.instances)
    }
}
impl LiveRack {
    pub fn set_bypass(&mut self, node: usize, bypass: bool) -> Result<(), HostError> {
        if node >= self.instances.len() {
            return Err("unknown bypass node".into());
        }
        if bypass {
            self.shared.bypass.fetch_or(1 << node, Ordering::Release);
        } else {
            self.shared
                .bypass
                .fetch_and(!(1 << node), Ordering::Release);
        }
        Ok(())
    }
}
/// Per-node dry path alignment; ticks continuously so bypass doesn't lose history.
pub(crate) struct BypassLine {
    data: Vec<f32>,
    at: usize,
}
impl BypassLine {
    pub(crate) fn new(frames: usize) -> Result<Self, HostError> {
        if frames > 1_048_576 {
            return Err("plug-in bypass latency exceeds limit".into());
        }
        Ok(Self {
            data: vec![0.0; frames * 2],
            at: 0,
        })
    }
    pub(crate) fn apply(&mut self, dry: &[f32], wet: &mut [f32], bypass: bool) {
        for (input, output) in dry.iter().zip(wet) {
            let delayed = if self.data.is_empty() {
                *input
            } else {
                let x = self.data[self.at];
                self.data[self.at] = *input;
                self.at = (self.at + 1) % self.data.len();
                x
            };
            if bypass {
                *output = delayed;
            }
        }
    }
}

impl ClapInstance {
    /// Restore an already validated parameter-only session while deactivated.
    pub fn restore_parameters(&mut self, values: &BTreeMap<u32, f64>) -> Result<(), HostError> {
        if self.active {
            return Err("deactivate before parameter restore".into());
        }
        for (id, value) in values {
            let p = self
                .parameters
                .iter()
                .find(|p| p.id == *id)
                .ok_or("unknown saved parameter")?;
            if !value.is_finite()
                || !(p.min..=p.max).contains(value)
                || p.flags & CLAP_PARAM_IS_READONLY != 0
            {
                return Err("invalid saved parameter".into());
            }
        }
        if values.is_empty() {
            return Ok(());
        }
        let ext = unsafe {
            (*self.plugin).get_extension.expect("checked")(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>()
        };
        if ext.is_null() {
            return Err("parameter restore extension unavailable".into());
        }
        let flush = unsafe { (*ext).flush }.ok_or("parameter restore flush unavailable")?;
        let events: Vec<_> = values
            .iter()
            .map(|(id, value)| NativeEvent {
                param: clap_event_param_value {
                    header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, 0),
                    param_id: *id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: *value,
                },
            })
            .collect();
        for chunk in events.chunks(64) {
            let pointers: Vec<*const clap_event_header> = chunk
                .iter()
                .map(|e| (e as *const NativeEvent).cast())
                .collect();
            let ie = clap_input_events {
                ctx: (&pointers as *const Vec<*const clap_event_header>)
                    .cast_mut()
                    .cast(),
                size: Some(event_size),
                get: Some(event_get),
            };
            let mut output = Vec::with_capacity(LIMIT);
            let mut dropped = 0;
            let mut context = OutputContext {
                events: &mut output,
                frames: 1,
                dropped: &mut dropped,
            };
            let oe = clap_output_events {
                ctx: (&mut context as *mut OutputContext<'_>).cast(),
                try_push: Some(event_push),
            };
            unsafe {
                flush(self.plugin, &ie, &oe);
            }
        }
        self.inspect()?;
        Ok(())
    }
}

impl LiveRack {
    pub fn is_bypassed(&self, node: usize) -> bool {
        self.shared.bypass.load(Ordering::Acquire) & (1 << node.min(31)) != 0
    }
    /// Apply supported floating editor preferences; unsupported size/scale are warnings.
    pub fn restore_editors(&mut self, session: &Session, indices: &[usize]) -> Vec<String> {
        let mut warnings = vec![];
        for (node, index) in indices.iter().enumerate() {
            let Some(slot) = session.slots.get(*index) else {
                warnings.push("invalid editor mapping".into());
                continue;
            };
            if !slot.editor.open {
                continue;
            }
            let result = if slot.editor.embedded {
                self.show_embedded_editor(node, &slot.id)
            } else {
                self.show_editor(node)
            };
            if let Err(e) = result {
                warnings.push(e.to_string());
                continue;
            }
            if let Err(e) = self.scale_editor(node, slot.editor.scale) {
                warnings.push(format!("{}: {e}", slot.id));
            }
            if let Some(size) = slot.editor.size {
                if let Err(e) = self.resize_editor(node, size) {
                    warnings.push(format!("{}: {e}", slot.id));
                }
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Session {
        let bytes = b"opaque";
        Session {
            schema: 1,
            name: "Test".into(),
            slots: vec![Slot {
                id: "one".into(),
                entry: RackEntry {
                    path: "missing.clap".into(),
                    plugin_id: "test".into(),
                },
                plugin_version: "1".into(),
                bypass: false,
                editor: Default::default(),
                parameters: BTreeMap::new(),
                state_hex: Some(encode(bytes)),
                state_sha256: Some(encode(&Sha256::digest(bytes))),
            }],
        }
    }
    #[test]
    fn state_corruption_and_duplicate_slots_rejected() {
        let mut d = document();
        assert!(Session::from_json(&d.to_json().unwrap()).is_ok());
        d.slots[0].state_hex = Some("ff".into());
        assert!(d.validate().is_err());
        let mut d = document();
        d.slots.push(d.slots[0].clone());
        assert!(d.validate().is_err());
    }
    #[test]
    fn merge_preserves_missing_payload_and_is_transactional() {
        let mut d = document();
        let mut missing = d.slots[0].clone();
        missing.id = "missing".into();
        d.slots.push(missing);
        let before = d.to_json().unwrap();
        assert!(d.merge_capture(document(), &[9]).is_err());
        assert_eq!(d.to_json().unwrap(), before);
        let mut capture = document();
        capture.slots[0].bypass = true;
        d.merge_capture(capture, &[0]).unwrap();
        assert!(d.slots[0].bypass);
        assert_eq!(d.slots[1].state_hex, Some(encode(b"opaque")));
    }
    #[test]
    fn bypass_delay_tracks_history_when_not_bypassed() {
        let mut line = BypassLine::new(1).unwrap();
        let mut wet = [9.0; 4];
        line.apply(&[1.0, 2.0, 3.0, 4.0], &mut wet, false);
        assert_eq!(wet, [9.0; 4]);
        line.apply(&[5.0, 6.0, 7.0, 8.0], &mut wet, true);
        assert_eq!(wet, [3.0, 4.0, 5.0, 6.0]);
        assert!(BypassLine::new(1_048_577).is_err());
    }
}
