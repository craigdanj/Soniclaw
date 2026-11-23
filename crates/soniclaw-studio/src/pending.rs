//! Bounded crash journals for uncommitted performances; no project is changed automatically.
use crate::{automation_write, midi_recording, project::ProjectError};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema: u32,
    pub project_id: String,
    pub midi: Option<midi_recording::Take>,
    pub automation: Option<automation_write::Checkpoint>,
}
impl Checkpoint {
    pub fn save(&self, root: &Path) -> Result<u64, ProjectError> {
        if self.schema != 1 || !crate::valid_id(&self.project_id) {
            return Err("invalid pending identity".into());
        }
        if let Some(t) = &self.midi {
            t.validate()?;
            if t.project_id != self.project_id || !t.interrupted {
                return Err("pending MIDI identity/state mismatch".into());
            }
        }
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("pending performance exceeds 32 MiB".into());
        }
        match std::fs::create_dir(root) {
            Ok(()) => {
                let mut f = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(root.join("project-id"))?;
                f.write_all(self.project_id.as_bytes())?;
                f.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        if std::fs::symlink_metadata(root)?.file_type().is_symlink()
            || std::fs::read_to_string(root.join("project-id"))? != self.project_id
        {
            return Err("pending journal belongs to another project".into());
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("writer.lock"))?;
        lock.try_lock()?;
        let old = entries(root)?;
        let generation = old.first().map_or(Ok(1), |(g, _)| {
            g.checked_add(1).ok_or("pending generation exhausted")
        })?;
        let path = root.join(format!("pending-{generation:020}.json"));
        let mut f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        for (_, name) in old.into_iter().skip(7) {
            std::fs::remove_file(root.join(name))?;
        }
        Ok(generation)
    }
    /// Newest well-formed checkpoint; corrupt interrupted writes are reported, then older entries are tried.
    pub fn load(root: &Path, project_id: &str) -> Result<(Self, Vec<String>), ProjectError> {
        if std::fs::read_to_string(root.join("project-id"))? != project_id {
            return Err("pending journal belongs to another project".into());
        }
        let mut warnings = vec![];
        for (_, name) in entries(root)? {
            let result = (|| -> Result<Self, ProjectError> {
                let path = root.join(&name);
                if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                    return Err("pending file is symlink".into());
                }
                let mut b = vec![];
                std::fs::File::open(path)?
                    .take(32 * 1024 * 1024 + 1)
                    .read_to_end(&mut b)?;
                if b.len() > 32 * 1024 * 1024 {
                    return Err("pending file too large".into());
                }
                let c: Self = serde_json::from_slice(&b)?;
                if c.schema != 1 || c.project_id != project_id {
                    return Err("pending identity/schema mismatch".into());
                }
                if let Some(t) = &c.midi {
                    t.validate()?;
                    if t.project_id != project_id || !t.interrupted {
                        return Err("invalid interrupted take".into());
                    }
                }
                Ok(c)
            })();
            match result {
                Ok(c) => return Ok((c, warnings)),
                Err(e) => warnings.push(format!("Rejected {name}: {e}")),
            }
        }
        Err(format!("no valid pending performance: {}", warnings.join("; ")).into())
    }
}
fn entries(root: &Path) -> Result<Vec<(u64, String)>, ProjectError> {
    let mut result = vec![];
    let mut count = 0;
    for e in std::fs::read_dir(root)? {
        count += 1;
        if count > 64 {
            return Err("pending journal entry limit".into());
        }
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(n) = name
            .strip_prefix("pending-")
            .and_then(|s| s.strip_suffix(".json"))
            .filter(|s| s.len() == 20 && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u64>().ok())
        {
            result.push((n, name));
        }
    }
    result.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_and_torn_write_fallback() {
        let root = std::env::temp_dir().join(format!("soniclaw-pending73-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = Checkpoint {
            schema: 1,
            project_id: "test".into(),
            midi: None,
            automation: None,
        };
        for _ in 0..12 {
            c.save(&root).unwrap();
        }
        assert_eq!(entries(&root).unwrap().len(), 8);
        std::fs::write(root.join("pending-00000000000000000013.json"), b"{").unwrap();
        let (_, warnings) = Checkpoint::load(&root, "test").unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(Checkpoint::load(&root, "other").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
