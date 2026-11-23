//! Bounded metadata cache and supervised trusted-module discovery. Not a security sandbox.
use crate::{
    formats::{self, PluginFormat, Product},
    HostError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub product: Product,
    pub fingerprint: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub reason: String,
    pub fingerprint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema: u32,
    pub entries: Vec<Entry>,
    pub quarantine: BTreeMap<PathBuf, Failure>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            schema: 1,
            entries: vec![],
            quarantine: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub format: PluginFormat,
    pub module: PathBuf,
    pub fingerprint: String,
    pub products: Vec<Product>,
    pub error: Option<String>,
    pub milliseconds: u128,
}
impl Registry {
    pub fn validate(&self) -> Result<(), HostError> {
        if self.schema != 1 || self.entries.len() > 4096 || self.quarantine.len() > 256 {
            return Err("registry limits exceeded".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in &self.entries {
            let p = &e.product;
            if !p.path.is_absolute()
                || p.path.as_os_str().len() > 4096
                || p.id.is_empty()
                || p.id.len() > 256
                || p.name.len() > 512
                || p.vendor.len() > 512
                || p.version.len() > 256
                || e.fingerprint.len() != 64
                || !e.fingerprint.bytes().all(|x| x.is_ascii_hexdigit())
                || !seen.insert((p.path.clone(), format!("{:?}", p.format), p.id.clone()))
            {
                return Err("invalid cached product".into());
            }
        }
        for (p, f) in &self.quarantine {
            if !p.is_absolute()
                || p.as_os_str().len() > 4096
                || f.reason.len() > 2048
                || f.fingerprint.len() != 64
            {
                return Err("invalid quarantine record".into());
            }
        }
        Ok(())
    }
    pub fn apply(&mut self, r: Report) -> Result<(), HostError> {
        let mut next = self.clone();
        if let Some(reason) = r.error {
            next.quarantine.insert(
                r.module,
                Failure {
                    reason,
                    fingerprint: r.fingerprint,
                },
            );
        } else {
            if r.products
                .iter()
                .any(|p| p.path != r.module || p.format != r.format)
            {
                return Err("scan product identity mismatch".into());
            }
            let old = self.entries.clone();
            next.entries
                .retain(|e| e.product.path != r.module || e.product.format != r.format);
            for p in r.products {
                let enabled = old
                    .iter()
                    .find(|e| {
                        e.product.path == p.path
                            && e.product.format == p.format
                            && e.product.id == p.id
                    })
                    .is_none_or(|e| e.enabled);
                next.entries.push(Entry {
                    product: p,
                    fingerprint: r.fingerprint.clone(),
                    enabled,
                });
            }
            next.quarantine.remove(&r.module);
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
    pub fn usable(&self, index: usize) -> Result<&Entry, HostError> {
        self.validate()?;
        let e = self.entries.get(index).ok_or("unknown product")?;
        if !e.enabled
            || self.quarantine.contains_key(&e.product.path)
            || fingerprint(&e.product.path)? != e.fingerprint
        {
            return Err("disabled, quarantined, missing or changed module; rescan first".into());
        }
        Ok(e)
    }
    pub fn read(path: &Path) -> Result<Self, HostError> {
        let bytes = read_bounded(path, 4 * 1024 * 1024)?;
        let r: Self = serde_json::from_slice(&bytes)?;
        r.validate()?;
        Ok(r)
    }
    pub fn write(&self, path: &Path) -> Result<(), HostError> {
        self.validate()?;
        let b = serde_json::to_vec_pretty(self)?;
        if b.len() > 4 * 1024 * 1024 {
            return Err("registry exceeds 4 MiB".into());
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), nonce));
        let r = (|| -> Result<(), HostError> {
            let mut f = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            f.write_all(&b)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&temp, path)?;
            Ok(())
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        r
    }
}
fn read_bounded(p: &Path, max: usize) -> Result<Vec<u8>, HostError> {
    let mut b = Vec::new();
    std::fs::File::open(p)?
        .take(max as u64 + 1)
        .read_to_end(&mut b)?;
    if b.len() > max {
        return Err("metadata exceeds bound".into());
    }
    Ok(b)
}
/// Hash one explicit module or bundle. Symlinks are rejected; never follows arbitrary search roots.
pub fn fingerprint(path: &Path) -> Result<String, HostError> {
    fn walk(root: &Path, p: &Path, files: &mut Vec<PathBuf>) -> Result<(), HostError> {
        let m = std::fs::symlink_metadata(p)?;
        if m.file_type().is_symlink() {
            return Err("module bundle symlinks unsupported".into());
        }
        if m.is_dir() {
            for e in std::fs::read_dir(p)? {
                walk(root, &e?.path(), files)?;
            }
        } else if m.is_file() {
            if files.len() >= 1024 {
                return Err("module exceeds 1024 files".into());
            }
            files.push(p.strip_prefix(root)?.into());
        } else {
            return Err("unsupported module resource".into());
        }
        Ok(())
    }
    let root = if path.is_dir() {
        path
    } else {
        path.parent().ok_or("missing module parent")?
    };
    let mut files = Vec::new();
    walk(root, path, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 65536];
    for f in files {
        let mut file = std::fs::File::open(root.join(&f))?;
        let n = file.metadata()?.len();
        total = total.checked_add(n).ok_or("module size overflow")?;
        if total > 512 * 1024 * 1024 {
            return Err("module exceeds 512 MiB".into());
        }
        let name = f.to_string_lossy();
        h.update((name.len() as u64).to_le_bytes());
        h.update(name.as_bytes());
        h.update(n.to_le_bytes());
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
/// # Safety
/// Loads caller-selected trusted native code. Invoke only in a supervised helper process.
pub unsafe fn child(format: PluginFormat, module: &Path, output: &Path) -> Result<(), HostError> {
    let products = unsafe { formats::products(format, module) }?;
    let b = serde_json::to_vec(&products)?;
    if b.len() > 1024 * 1024 {
        return Err("scan output too large".into());
    }
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?;
    f.write_all(&b)?;
    Ok(())
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// # Safety
/// The helper and selected native module/dependencies must be explicitly trusted. Supervision contains scan crashes/hangs only.
pub unsafe fn scan(
    helper: &Path,
    module: &Path,
    format: PluginFormat,
    timeout: Duration,
) -> Result<Report, HostError> {
    if !(Duration::from_millis(100)..=Duration::from_secs(30)).contains(&timeout) {
        return Err("scan timeout must be 0.1..30 seconds".into());
    }
    let module = module.canonicalize()?;
    let hash = fingerprint(&module)?;
    let start = Instant::now();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("soniclaw-scan-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let _folder = Folder(dir.clone());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let out = dir.join("products.json");
    let mut cmd = Command::new(helper);
    cmd.arg("--native-scan-child")
        .arg(if format == PluginFormat::Clap {
            "clap"
        } else {
            "vst3"
        })
        .arg(&module)
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = Child(cmd.spawn()?);
    let result = (|| -> Result<Vec<Product>, HostError> {
        loop {
            if let Some(status) = child.0.try_wait()? {
                if !status.success() {
                    return Err(format!("scanner exited {status}").into());
                }
                break;
            }
            if start.elapsed() >= timeout {
                return Err("scan timed out; helper terminated".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let b = read_bounded(&out, 1024 * 1024)?;
        let p: Vec<Product> = serde_json::from_slice(&b)?;
        if p.len() > 4096 || fingerprint(&module)? != hash {
            return Err("module changed or too many products".into());
        }
        Ok(p)
    })();
    let (products, error) = match result {
        Ok(p) => (p, None),
        Err(e) => (vec![], Some(e.to_string().chars().take(2048).collect())),
    };
    Ok(Report {
        format,
        module,
        fingerprint: hash,
        products,
        error,
        milliseconds: start.elapsed().as_millis(),
    })
}
pub fn descriptor(e: &Entry, id: &str, instrument: bool) -> soniclaw_studio::Descriptor {
    let p = &e.product;
    let kind = if p.format == PluginFormat::Clap {
        "clap"
    } else {
        "vst3"
    };
    let mut state = serde_json::json!({"path":p.path,"module_sha256":e.fingerprint,"role":if instrument{"instrument"}else{"effect"},"version":p.version});
    state[if kind == "clap" {
        "plugin_id"
    } else {
        "class_id"
    }] = serde_json::json!(p.id);
    soniclaw_studio::Descriptor {
        id: id.into(),
        kind: kind.into(),
        state,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quarantine_requires_successful_rescan() {
        let p = PathBuf::from("/owned.clap");
        let mut r = Registry::default();
        let report = Report {
            format: PluginFormat::Clap,
            module: p.clone(),
            fingerprint: "0".repeat(64),
            products: vec![],
            error: Some("timeout".into()),
            milliseconds: 100,
        };
        r.apply(report.clone()).unwrap();
        assert!(r.quarantine.contains_key(&p));
        let mut retry = report;
        retry.error = None;
        r.apply(retry).unwrap();
        assert!(r.quarantine.is_empty());
    }
    #[test]
    fn malformed_cache_rejected() {
        let r = Registry {
            schema: 5,
            ..Default::default()
        };
        assert!(r.validate().is_err());
    }
}
