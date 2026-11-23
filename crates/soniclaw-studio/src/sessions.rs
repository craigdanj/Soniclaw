//! Immutable save generations with a checked, atomically replaced project head. Worker thread only.
use crate::project::{Project, ProjectError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
const HEAD: &str = "soniclaw-session.json";
const STORE: &str = "session-store";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
    project_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    schema: u32,
    project_id: String,
    generation: u64,
    sha256: String,
}
#[derive(Clone, Debug)]
pub struct Saved {
    pub directory: PathBuf,
    pub generation: u64,
}
fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ProjectError> {
    let mut b = vec![];
    std::fs::File::open(path)?.take(16385).read_to_end(&mut b)?;
    if b.len() > 16384 {
        return Err("session metadata exceeds 16 KiB".into());
    }
    Ok(serde_json::from_slice(&b)?)
}
fn hash(path: &Path) -> Result<String, ProjectError> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut b = [0; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn owned(path: &Path) -> Result<(), ProjectError> {
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err("session metadata cannot be a symlink".into());
    }
    Ok(())
}
/// Resolve a committed generation, or a legacy standalone portable project folder.
pub fn current_directory(directory: &Path) -> Result<PathBuf, ProjectError> {
    let root = directory.canonicalize()?;
    let path = root.join(HEAD);
    if !path.try_exists()? {
        return Ok(root);
    }
    owned(&path)?;
    let h: Head = read(&path)?;
    let store = root.join(STORE);
    owned(&store)?;
    let m: Marker = read(&store.join("store.json"))?;
    if h.schema != 1
        || m.schema != 1
        || h.project_id != m.project_id
        || !crate::valid_id(&h.project_id)
        || h.generation == 0
    {
        return Err("invalid session head/identity".into());
    }
    let generation = store.join(format!("generation-{:020}", h.generation));
    owned(&generation)?;
    let generation = generation.canonicalize()?;
    if !generation.starts_with(store.canonicalize()?)
        || hash(&generation.join("project.json"))? != h.sha256
    {
        return Err("session generation escapes store or manifest changed".into());
    }
    Ok(generation)
}
fn sync_tree(path: &Path) -> Result<(), ProjectError> {
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let ty = e.file_type()?;
        if ty.is_dir() {
            sync_tree(&e.path())?;
        } else if ty.is_file() {
            std::fs::File::open(e.path())?.sync_all()?;
        } else {
            return Err("unsupported saved resource type".into());
        }
    }
    #[cfg(unix)]
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}
/// Save repeatedly into an owned project root. Existing generations and raw legacy media are retained.
/// Failed saves never change the previous head. Do not prune generations while projects/history use them.
pub fn save(project: &Project, directory: &Path) -> Result<Saved, ProjectError> {
    project.validate()?;
    match std::fs::create_dir(directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let root = directory.canonicalize()?;
    let store = root.join(STORE);
    if !store.try_exists()? {
        if root.join("project.json").exists() {
            if Project::open(&root)?.project_id != project.project_id {
                return Err("destination belongs to another project".into());
            }
        } else if std::fs::read_dir(&root)?.next().is_some() {
            return Err("destination is not an empty or recognized project folder".into());
        }
        std::fs::create_dir(&store)?;
        let mut marker = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(store.join("store.json"))?;
        marker.write_all(&serde_json::to_vec(&Marker {
            schema: 1,
            project_id: project.project_id.clone(),
        })?)?;
        marker.sync_all()?;
    }
    owned(&store)?;
    owned(&store.join("store.json"))?;
    let marker: Marker = read(&store.join("store.json"))?;
    if marker.schema != 1 || marker.project_id != project.project_id {
        return Err("session store belongs to another project".into());
    }
    let lock_path = store.join("writer.lock");
    if lock_path.exists() {
        owned(&lock_path)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock()
        .map_err(|e| format!("session writer busy: {e}"))?;
    if root.join(HEAD).exists() {
        current_directory(&root)?;
    }
    let mut generation = 0;
    let mut count = 0;
    for e in std::fs::read_dir(&store)? {
        count += 1;
        if count > 130 {
            return Err("save generation limit; consolidate to a new project folder".into());
        }
        let e = e?;
        if let Some(n) = e
            .file_name()
            .to_str()
            .and_then(|s| s.strip_prefix("generation-"))
            .and_then(|s| s.parse::<u64>().ok())
        {
            generation = generation.max(n);
        }
    }
    generation = generation.checked_add(1).ok_or("generation exhausted")?;
    let path = store.join(format!("generation-{generation:020}"));
    project.save_new(&path)?;
    let temporary = root.join(format!("session-head-{generation:020}.tmp"));
    let result = (|| -> Result<(), ProjectError> {
        sync_tree(&path)?;
        let head = Head {
            schema: 1,
            project_id: project.project_id.clone(),
            generation,
            sha256: hash(&path.join("project.json"))?,
        };
        let mut f = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        f.write_all(&serde_json::to_vec_pretty(&head)?)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&temporary, root.join(HEAD))?;
        #[cfg(unix)]
        std::fs::File::open(&root)?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    Ok(Saved {
        directory: path,
        generation,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_save_stale_identity_failed_resource_and_verified_open() {
        let root = std::env::temp_dir().join(format!("soniclaw-session73-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut p = crate::workspace::blank("Initial".into(), 48000, 128).unwrap();
        assert_eq!(save(&p, &root).unwrap().generation, 1);
        p.name = "Second".into();
        let second = save(&p, &root).unwrap();
        assert_eq!(Project::open(&root).unwrap().name, "Second");
        assert_eq!(
            Project::open(&root.join(STORE).join("generation-00000000000000000001"))
                .unwrap()
                .name,
            "Initial"
        );
        let other = crate::workspace::blank("Other".into(), 48000, 128).unwrap();
        assert!(save(&other, &root).is_err());
        p.disk.insert("missing".into(), root.join("absent.wav"));
        assert!(save(&p, &root).is_err());
        assert_eq!(Project::open(&root).unwrap().name, "Second");
        std::fs::write(second.directory.join("project.json"), b"{}").unwrap();
        assert!(Project::open(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
