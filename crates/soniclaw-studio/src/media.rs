//! Read-only dependency auditing and nondestructive consolidated project copies.
use crate::{
    project::{Project, ProjectError},
    *,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Storage {
    Memory,
    Disk,
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub storage: Storage,
    pub bytes: u64,
    pub referenced: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub project_id: String,
    pub entries: Vec<Entry>,
    pub removable_memory_bytes: u64,
    pub removable_disk_bytes: u64,
    pub opaque_dependencies: bool,
    pub warnings: Vec<String>,
}
#[derive(Clone)]
pub struct Plan {
    report: Report,
    fingerprint: String,
}
impl Plan {
    pub fn report(&self) -> &Report {
        &self.report
    }
}
fn collect(d: &Document, memory: &mut BTreeSet<String>, disk: &mut BTreeSet<String>) -> bool {
    for t in &d.tracks {
        for c in &t.audio {
            memory.insert(c.asset.clone());
        }
        for c in &t.streamed {
            disk.insert(c.asset.clone());
        }
    }
    let mut opaque = false;
    for p in d
        .tracks
        .iter()
        .flat_map(|t| t.instrument.iter().chain(&t.inserts))
        .chain(d.auxiliaries.iter().flat_map(|a| &a.inserts))
        .chain(d.mix_routing.descriptors())
    {
        match p.kind.as_str() {
            "stock.sampler" => {
                if let Some(id) = p.state["asset"].as_str() {
                    memory.insert(id.into());
                } else {
                    opaque = true;
                }
            }
            "gain" | "lowpass" | "delay" | "sine" | "saw" | "stock.fx" | "stock.synth"
            | "stock.sidechain" => {}
            _ => opaque = true,
        }
    }
    opaque
}
fn fingerprint(p: &Project) -> Result<String, ProjectError> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&(
        &p.native_states,
        &p.project_id,
        &p.name,
        &p.document,
        &p.freezes,
        &p.takes,
        &p.midi_takes,
        &p.comps,
        &p.warps,
        &p.disk,
    ))?);
    let mut bytes = Vec::with_capacity(65536);
    for (id, a) in &p.assets {
        hash.update(id);
        hash.update(a.sample_rate.to_le_bytes());
        hash.update(a.channels.to_le_bytes());
        for chunk in a.samples.chunks(16384) {
            bytes.clear();
            for s in chunk {
                bytes.extend_from_slice(&s.to_bits().to_le_bytes());
            }
            hash.update(&bytes);
        }
    }
    let mut block = [0; 65536];
    for path in p.disk.values() {
        let mut file = std::fs::File::open(path)?;
        loop {
            let n = file.read(&mut block)?;
            if n == 0 {
                break;
            }
            hash.update(&block[..n]);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// Conservatively retains disabled regions, all take/freeze histories and opaque processor dependencies.
pub fn audit(p: &Project) -> Result<Plan, ProjectError> {
    p.validate()?;
    let mut memory = BTreeSet::new();
    let mut disk = BTreeSet::new();
    let mut opaque = collect(&p.document, &mut memory, &mut disk);
    for frozen in p.freezes.values() {
        opaque |= collect(&frozen.original, &mut memory, &mut disk);
    }
    if opaque {
        memory.extend(p.assets.keys().cloned());
        disk.extend(p.disk.keys().cloned());
    }
    loop {
        let mut changed = false;
        for (id, binding) in &p.warps {
            if disk.contains(id) {
                changed |= disk.insert(binding.source.clone());
            }
        }
        if !changed {
            break;
        }
    }
    for id in &memory {
        if !p.assets.contains_key(id) {
            return Err("referenced memory asset is missing".into());
        }
    }
    for id in &disk {
        if !p.disk.contains_key(id) {
            return Err("referenced disk asset is missing".into());
        }
    }
    let mut entries = vec![];
    let mut removable_memory_bytes = 0;
    let mut removable_disk_bytes = 0;
    for (id, a) in &p.assets {
        let referenced = memory.contains(id);
        let bytes = (a.samples.len() * 4) as u64;
        if !referenced {
            removable_memory_bytes += bytes;
        }
        entries.push(Entry {
            id: id.clone(),
            storage: Storage::Memory,
            bytes,
            referenced,
        });
    }
    for (id, path) in &p.disk {
        let referenced = disk.contains(id);
        let bytes = std::fs::metadata(path)?.len();
        if !referenced {
            removable_disk_bytes += bytes;
        }
        entries.push(Entry {
            id: id.clone(),
            storage: Storage::Disk,
            bytes,
            referenced,
        });
    }
    let mut warnings=vec!["Take folders and freeze histories are retained; their storage is outside direct-asset byte totals.".into()];
    if opaque {
        warnings.push("Custom/native processor state has opaque dependencies: all registered resources retained. External plug-in files are not automatically discovered/copied.".into());
    }
    Ok(Plan {
        report: Report {
            schema: 1,
            project_id: p.project_id.clone(),
            entries,
            removable_memory_bytes,
            removable_disk_bytes,
            opaque_dependencies: opaque,
            warnings,
        },
        fingerprint: fingerprint(p)?,
    })
}
#[derive(Debug, Serialize)]
pub struct Consolidated {
    pub schema: u32,
    pub source_project_id: String,
    pub retained_assets: usize,
    pub omitted_assets: usize,
    pub report: Report,
}
/// Creates a NEW portable folder; never removes any source file or edits the active project.
pub fn consolidate(
    p: &Project,
    plan: &Plan,
    destination: &Path,
) -> Result<Consolidated, ProjectError> {
    if destination.exists() {
        return Err("consolidation requires a new directory".into());
    }
    let current = audit(p)?;
    if current.fingerprint != plan.fingerprint {
        return Err("media audit is stale; run audit again".into());
    }
    let mut next = p.clone();
    let memory: BTreeSet<_> = current
        .report
        .entries
        .iter()
        .filter(|e| e.referenced && e.storage == Storage::Memory)
        .map(|e| e.id.clone())
        .collect();
    let disk: BTreeSet<_> = current
        .report
        .entries
        .iter()
        .filter(|e| e.referenced && e.storage == Storage::Disk)
        .map(|e| e.id.clone())
        .collect();
    next.assets.retain(|id, _| memory.contains(id));
    next.disk.retain(|id, _| disk.contains(id));
    next.warps.retain(|id, _| disk.contains(id));
    next.validate()?;
    next.save_new(destination)?;
    let result = (|| -> Result<Consolidated, ProjectError> {
        let opened = Project::open(destination)?;
        opened.validate()?;
        if fingerprint(p)? != current.fingerprint {
            return Err("source media changed during consolidation".into());
        }
        let report = Consolidated {
            schema: 1,
            source_project_id: p.project_id.clone(),
            retained_assets: next.assets.len() + next.disk.len(),
            omitted_assets: p.assets.len() + p.disk.len() - next.assets.len() - next.disk.len(),
            report: current.report,
        };
        std::fs::write(
            destination.join("media-audit.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(report)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reachability_keeps_disabled_and_opaque_resources() {
        let (d, a) = crate::tests::fixture();
        let mut p = Project::new("Audit".into(), d, a, Default::default()).unwrap();
        p.assets.insert("unused".into(), p.assets["a"].clone());
        p.document.tracks[0].audio[0].enabled = false;
        let report = audit(&p).unwrap();
        assert!(
            report
                .report
                .entries
                .iter()
                .find(|e| e.id == "a")
                .unwrap()
                .referenced
        );
        assert!(
            !report
                .report
                .entries
                .iter()
                .find(|e| e.id == "unused")
                .unwrap()
                .referenced
        );
        p.document.tracks[0].inserts[0].kind = "custom".into();
        let report = audit(&p).unwrap();
        assert!(report.report.opaque_dependencies);
        assert!(report.report.entries.iter().all(|e| e.referenced));
    }
}
