//! Verified opaque native resources. Reading these values never loads native code.
use crate::{
    project::{Project, ProjectError},
    Descriptor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub type States = BTreeMap<String, Blob>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub format: String,
    pub module: PathBuf,
    pub product: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub schema: u32,
    pub identity: Identity,
    pub parameters: BTreeMap<u32, f64>,
    pub opaque: bool,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Blob {
    #[serde(flatten)]
    metadata: Metadata,
    #[serde(skip)]
    bytes: Arc<[u8]>,
}
impl Blob {
    pub fn new(
        identity: Identity,
        parameters: BTreeMap<u32, f64>,
        bytes: Option<Vec<u8>>,
    ) -> Result<Self, ProjectError> {
        let opaque = bytes.is_some();
        let bytes = bytes.unwrap_or_default();
        let m = Metadata {
            schema: 1,
            identity,
            parameters,
            opaque,
            bytes: bytes.len(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        Self::from_stored(m, bytes)
    }
    pub fn from_stored(metadata: Metadata, bytes: Vec<u8>) -> Result<Self, ProjectError> {
        if metadata.schema != 1
            || !matches!(metadata.identity.format.as_str(), "clap" | "vst3")
            || !metadata.identity.module.is_absolute()
            || metadata.identity.module.as_os_str().len() > 4096
            || metadata.identity.product.is_empty()
            || metadata.identity.product.len() > 256
            || metadata.identity.version.len() > 256
            || metadata.parameters.len() > 65536
            || metadata.parameters.values().any(|v| !v.is_finite())
            || bytes.len() > MAX_BYTES
            || metadata.bytes != bytes.len()
            || !metadata.opaque && !bytes.is_empty()
            || metadata.sha256 != format!("{:x}", Sha256::digest(&bytes))
        {
            return Err("invalid native snapshot identity, size, parameters or digest".into());
        }
        Ok(Self {
            metadata,
            bytes: bytes.into(),
        })
    }
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn rebind(&self, module: PathBuf) -> Result<Self, ProjectError> {
        let mut m = self.metadata.clone();
        m.identity.module = module;
        Self::from_stored(m, self.bytes.to_vec())
    }
    pub fn matches(&self, d: &Descriptor) -> bool {
        let m = &self.metadata.identity;
        m.format == d.kind
            && d.state["path"]
                .as_str()
                .is_some_and(|s| std::path::Path::new(s) == m.module)
            && d.state[if d.kind == "clap" {
                "plugin_id"
            } else {
                "class_id"
            }]
            .as_str()
                == Some(&m.product)
    }
}
pub fn descriptors(d: &crate::Document) -> impl Iterator<Item = &Descriptor> {
    d.tracks
        .iter()
        .flat_map(|t| t.instrument.iter().chain(&t.inserts))
        .chain(d.auxiliaries.iter().flat_map(|a| &a.inserts))
        .chain(d.mix_routing.descriptors())
}
pub fn descriptors_mut(d: &mut crate::Document) -> impl Iterator<Item = &mut Descriptor> {
    d.tracks
        .iter_mut()
        .flat_map(|t| t.instrument.iter_mut().chain(&mut t.inserts))
        .chain(d.auxiliaries.iter_mut().flat_map(|a| &mut a.inserts))
        .chain(d.mix_routing.buses.iter_mut().flat_map(|b| &mut b.inserts))
        .chain(&mut d.mix_routing.master.inserts)
}
pub fn validate(p: &Project) -> Result<(), ProjectError> {
    if p.native_states.len() > 256
        || p.native_states
            .values()
            .map(|b| b.bytes.len())
            .sum::<usize>()
            > 128 * 1024 * 1024
        || p.native_states
            .values()
            .map(|b| b.metadata.parameters.len())
            .sum::<usize>()
            > 65536
        || p.native_states.keys().any(|id| !crate::valid_id(id))
    {
        return Err("native snapshot project limits exceeded".into());
    }
    for d in
        descriptors(&p.document).chain(p.freezes.values().flat_map(|f| descriptors(&f.original)))
    {
        if matches!(d.kind.as_str(), "clap" | "vst3")
            && d.state["native_state"].as_bool() == Some(true)
            && !p.native_states.get(&d.id).is_some_and(|b| b.matches(d))
        {
            return Err("missing or mismatched native snapshot".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_metadata_and_digest() {
        let b = Blob::new(
            Identity {
                format: "clap".into(),
                module: PathBuf::from("/fixture.clap"),
                product: "gain".into(),
                version: "1".into(),
            },
            BTreeMap::from([(7, 0.3)]),
            Some(vec![42; 131072]),
        )
        .unwrap();
        assert!(serde_json::to_vec(&b).unwrap().len() < 1024);
        assert!(Blob::from_stored(b.metadata.clone(), vec![0; 131072]).is_err());
        assert_eq!(
            b.rebind(PathBuf::from("/moved.clap")).unwrap().bytes(),
            b.bytes()
        );
    }
}
