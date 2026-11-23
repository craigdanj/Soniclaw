//! Bounded portable bundles and opt-in recovery snapshots. Control threads only.
//! Digests detect accidental corruption, not the identity of a file's author.
use crate::state::{ProjectState, StateError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;
const ASSET_LIMIT: u64 = 128 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 256 * 1024 * 1024;
static SERIAL: AtomicU64 = AtomicU64::new(0);
fn unique(prefix: &str) -> String {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}{time:039}-{}-{:020}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, StateError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err("file exceeds size limit".into());
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err("file exceeds size limit".into());
    }
    Ok(data)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Stores a project and its scoped WAV assets in one .soniclaw file.
/// Never replaces an existing destination. No audio-thread or plug-in code runs.
pub fn pack_bundle(
    state: &ProjectState,
    root: &Path,
    destination: &Path,
) -> Result<(), StateError> {
    state.validate(&state.product_id)?;
    if destination.exists() {
        return Err("bundle destination exists".into());
    }
    if state.assets.len() > 1024 {
        return Err("too many bundle assets".into());
    }
    let mut portable = state.clone();
    let sources = state
        .assets
        .iter()
        .map(|a| state.resolve_asset(root, a))
        .collect::<Result<Vec<_>, _>>()?;
    for (i, a) in portable.assets.iter_mut().enumerate() {
        a.path = format!("assets/{i}.wav");
    }
    let json = portable.to_json()?;
    if json.len() > MANIFEST_LIMIT {
        return Err("manifest exceeds limit".into());
    }
    let temp = parent(destination).join(unique(".soniclaw-pack-"));
    let result = (|| -> Result<(), StateError> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(b"SLPK")?;
        f.write_all(&1u32.to_le_bytes())?;
        f.write_all(&(json.len() as u32).to_le_bytes())?;
        f.write_all(&(sources.len() as u32).to_le_bytes())?;
        f.write_all(&digest(json.as_bytes()))?;
        f.write_all(json.as_bytes())?;
        let mut total = 0u64;
        for source in sources {
            let data = read_bounded(&source, ASSET_LIMIT)?;
            total = total
                .checked_add(data.len() as u64)
                .ok_or("asset length overflow")?;
            if total > TOTAL_LIMIT {
                return Err("bundle assets exceed 256 MiB".into());
            }
            // Ensure assets are bounded WAV files, not arbitrary imported content.
            crate::read_audio_with_limit(&source, ASSET_LIMIT as usize)?;
            f.write_all(&(data.len() as u64).to_le_bytes())?;
            f.write_all(&digest(&data))?;
            f.write_all(&data)?;
        }
        f.sync_all()?;
        // Atomic no-replace publication on the same filesystem, including Windows.
        fs::hard_link(&temp, destination)?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

fn exact<const N: usize>(f: &mut File) -> Result<[u8; N], StateError> {
    let mut bytes = [0; N];
    f.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Verifies and extracts a bundle into a NEW directory. Only fixed generated
/// asset paths are accepted. project.json is published last as the commit marker.
/// On failure, this function removes only the destination it created.
pub fn unpack_bundle(bundle: &Path, destination: &Path) -> Result<ProjectState, StateError> {
    let mut f = File::open(bundle)?;
    if f.metadata()?.len() > TOTAL_LIMIT + MANIFEST_LIMIT as u64 + 48 + 1024 * 40 {
        return Err("bundle exceeds size limit".into());
    }
    if exact::<4>(&mut f)? != *b"SLPK" || u32::from_le_bytes(exact(&mut f)?) != 1 {
        return Err("unsupported bundle format".into());
    }
    let len = u32::from_le_bytes(exact(&mut f)?) as usize;
    let count = u32::from_le_bytes(exact(&mut f)?) as usize;
    if len > MANIFEST_LIMIT || count > 1024 {
        return Err("bundle header exceeds limits".into());
    }
    let hash = exact::<32>(&mut f)?;
    let mut json = vec![0; len];
    f.read_exact(&mut json)?;
    if digest(&json) != hash {
        return Err("manifest checksum mismatch".into());
    }
    let state = ProjectState::from_json(std::str::from_utf8(&json)?)?;
    state.validate(&state.product_id)?;
    if state.assets.len() != count
        || state
            .assets
            .iter()
            .enumerate()
            .any(|(i, a)| a.path != format!("assets/{i}.wav"))
    {
        return Err("bundle contains noncanonical asset paths".into());
    }
    // create_dir gives exclusive ownership: an existing project is never touched.
    fs::create_dir(destination)?;
    let result = (|| -> Result<(), StateError> {
        fs::create_dir(destination.join("assets"))?;
        let mut total = 0u64;
        for i in 0..count {
            let len = u64::from_le_bytes(exact(&mut f)?);
            total = total.checked_add(len).ok_or("asset length overflow")?;
            if len > ASSET_LIMIT || total > TOTAL_LIMIT {
                return Err("bundle asset exceeds limits".into());
            }
            let hash = exact::<32>(&mut f)?;
            let mut bytes = vec![0; len as usize];
            f.read_exact(&mut bytes)?;
            if digest(&bytes) != hash {
                return Err("asset checksum mismatch".into());
            }
            let path = destination.join(format!("assets/{i}.wav"));
            write_new(&path, &bytes)?;
            crate::read_audio_with_limit(&path, ASSET_LIMIT as usize)?;
        }
        if f.read(&mut [0; 1])? != 0 {
            return Err("trailing bundle data".into());
        }
        write_new(&destination.join("project.json"), &json)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(destination);
        return Err(e);
    }
    Ok(state)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverySnapshot {
    pub project: ProjectState,
    pub asset_root: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u32,
    digest: [u8; 32],
    payload: String,
}

/// Immutable, checksummed snapshots with bounded retention. Assets remain external.
/// Keep a store exclusive to one application session/product. File I/O is not realtime.
pub struct RecoveryStore {
    directory: PathBuf,
    retain: usize,
}
impl RecoveryStore {
    pub fn new(directory: impl Into<PathBuf>, retain: usize) -> Result<Self, StateError> {
        if !(1..=32).contains(&retain) {
            return Err("recovery retention must be 1..32".into());
        }
        let directory = directory.into();
        fs::create_dir_all(&directory)?;
        Ok(Self { directory, retain })
    }
    fn files(&self) -> Result<Vec<PathBuf>, StateError> {
        let mut files = Vec::new();
        for e in fs::read_dir(&self.directory)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if e.file_type()?.is_file()
                && name.starts_with("checkpoint-")
                && name.ends_with(".json")
            {
                files.push(e.path());
            }
        }
        files.sort();
        Ok(files)
    }
    /// Commit a new snapshot before pruning previous ones. Does not save user files.
    pub fn checkpoint(&self, state: &ProjectState, root: &Path) -> Result<PathBuf, StateError> {
        state.validate(&state.product_id)?;
        let payload = serde_json::to_string(&RecoverySnapshot {
            project: state.clone(),
            asset_root: root.canonicalize()?,
        })?;
        if payload.len() > MANIFEST_LIMIT {
            return Err("recovery payload exceeds 4 MiB".into());
        }
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            digest: digest(payload.as_bytes()),
            payload,
        })?;
        let name = unique("checkpoint-");
        let temp = self.directory.join(format!(".{name}.tmp"));
        let final_path = self.directory.join(format!("{name}.json"));
        let result = (|| -> Result<(), StateError> {
            write_new(&temp, &bytes)?;
            fs::hard_link(&temp, &final_path)?;
            Ok(())
        })();
        let _ = fs::remove_file(temp);
        result?;
        let files = self.files()?;
        let remove = files.len().saturating_sub(self.retain);
        for old in files.into_iter().take(remove) {
            fs::remove_file(old)?;
        }
        Ok(final_path)
    }
    /// Skip corrupt/incompatible newer snapshots, retaining the last usable one.
    pub fn latest(&self, expected_product: &str) -> Result<Option<RecoverySnapshot>, StateError> {
        for path in self.files()?.into_iter().rev() {
            let attempt = (|| -> Result<RecoverySnapshot, StateError> {
                let bytes = read_bounded(&path, (MANIFEST_LIMIT * 2 + 1024) as u64)?;
                let e: Envelope = serde_json::from_slice(&bytes)?;
                if e.version != 1
                    || e.payload.len() > MANIFEST_LIMIT
                    || digest(e.payload.as_bytes()) != e.digest
                {
                    return Err("invalid recovery envelope".into());
                }
                let s: RecoverySnapshot = serde_json::from_str(&e.payload)?;
                s.project.validate(expected_product)?;
                Ok(s)
            })();
            if let Ok(s) = attempt {
                return Ok(Some(s));
            }
        }
        Ok(None)
    }
}
