//! Preset metadata, immutable project documents and scoped asset relocation.
//! All catalog operations allocate/access files: use a control or resource worker.
use crate::{core::*, state::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &Path, max: u64) -> Result<Vec<u8>, StateError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err("catalog file exceeds limit".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn digest_name(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PresetEntry {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub bank: String,
    pub favourite: bool,
    pub project_file: String,
    pub asset_root: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    generation: u64,
    entries: Vec<PresetEntry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    digest: String,
    payload: String,
}
fn validate(index: &Index) -> Result<(), StateError> {
    if index.version != 1 || index.entries.len() > 4096 {
        return Err("invalid catalog version/count".into());
    }
    let mut ids = BTreeSet::new();
    for e in &index.entries {
        let filename = e
            .project_file
            .strip_prefix("presets/")
            .and_then(|s| s.strip_suffix(".json"))
            .ok_or("invalid catalog project path")?;
        if !digest_name(&e.id)
            || !digest_name(filename)
            || !ids.insert(&e.id)
            || e.name.is_empty()
            || e.name.len() > 256
            || e.bank.len() > 128
            || e.tags.len() > 16
            || e.tags.iter().any(|s| s.is_empty() || s.len() > 64)
            || !e.asset_root.is_absolute()
        {
            return Err("invalid catalog entry".into());
        }
    }
    Ok(())
}
pub struct PresetCatalog {
    root: PathBuf,
    index: Index,
}
impl PresetCatalog {
    /// Dedicated writable catalog folder. Recover the highest valid immutable index.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StateError> {
        let root = root.into();
        fs::create_dir_all(root.join("presets"))?;
        let root = root.canonicalize()?;
        if !root.join("presets").canonicalize()?.starts_with(&root) {
            return Err("catalog preset directory escapes root".into());
        }
        let mut indexes = Vec::new();
        for e in fs::read_dir(&root)? {
            let e = e?;
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("catalog-") && n.ends_with(".json") && e.file_type()?.is_file() {
                indexes.push(e.path());
                if indexes.len() > 4096 {
                    return Err("too many catalog index files".into());
                }
            }
        }
        indexes.sort();
        let mut found = None;
        for p in indexes.iter().rev() {
            let candidate = (|| -> Result<Index, StateError> {
                let e: Envelope = serde_json::from_slice(&read(p, 16 * 1024 * 1024)?)?;
                if hash(e.payload.as_bytes()) != e.digest {
                    return Err("catalog checksum mismatch".into());
                }
                let i: Index = serde_json::from_str(&e.payload)?;
                validate(&i)?;
                Ok(i)
            })();
            if let Ok(i) = candidate {
                found = Some(i);
                break;
            }
        }
        if found.is_none() && !indexes.is_empty() {
            return Err("no valid catalog index; preserve files for recovery".into());
        }
        Ok(Self {
            root,
            index: found.unwrap_or(Index {
                version: 1,
                generation: 0,
                entries: vec![],
            }),
        })
    }
    pub fn entries(&self) -> &[PresetEntry] {
        &self.index.entries
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn entry(&self, id: &str) -> Result<&PresetEntry, StateError> {
        self.index
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| "unknown preset".into())
    }
    fn store_project(&self, s: &ProjectState) -> Result<String, StateError> {
        let bytes = s.to_json()?.into_bytes();
        let filename = format!("presets/{}.json", hash(&bytes));
        let path = self.root.join(&filename);
        if path.exists() {
            if read(&path, 4 * 1024 * 1024)? != bytes {
                return Err("immutable preset checksum conflict".into());
            }
        } else {
            write_new(&path, &bytes)?;
        }
        Ok(filename)
    }
    fn commit(&mut self, mut next: Index) -> Result<(), StateError> {
        validate(&next)?;
        next.generation = self
            .index
            .generation
            .checked_add(1)
            .ok_or("catalog generation exhausted")?;
        // Skip corrupt later generations without overwriting them during recovery.
        while self
            .root
            .join(format!("catalog-{:020}.json", next.generation))
            .exists()
        {
            next.generation = next
                .generation
                .checked_add(1)
                .ok_or("catalog generation exhausted")?;
        }
        let payload = serde_json::to_string(&next)?;
        let bytes = serde_json::to_vec(&Envelope {
            digest: hash(payload.as_bytes()),
            payload,
        })?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("catalog exceeds 16 MiB".into());
        }
        write_new(
            &self
                .root
                .join(format!("catalog-{:020}.json", next.generation)),
            &bytes,
        )?;
        self.index = next;
        // Keep the latest three index generations. Immutable projects are retained.
        let mut files: Vec<_> = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().is_some_and(|n| {
                    n.to_string_lossy().starts_with("catalog-")
                        && n.to_string_lossy().ends_with(".json")
                })
            })
            .collect();
        files.sort();
        let remove = files.len().saturating_sub(3);
        for p in files.into_iter().take(remove) {
            let _ = fs::remove_file(p);
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        state: &ProjectState,
        asset_root: &Path,
        tags: Vec<String>,
        bank: String,
    ) -> Result<String, StateError> {
        let filename = self.store_project(state)?;
        let id = hash(state.to_json()?.as_bytes());
        if self.index.entries.iter().any(|e| e.id == id) {
            return Err("preset already in catalog".into());
        }
        let mut next = self.index.clone();
        next.entries.push(PresetEntry {
            id: id.clone(),
            name: state.name.clone(),
            tags,
            bank,
            favourite: false,
            project_file: filename,
            asset_root: asset_root.canonicalize()?,
        });
        self.commit(next)?;
        Ok(id)
    }
    pub fn edit_metadata(
        &mut self,
        id: &str,
        name: String,
        tags: Vec<String>,
        bank: String,
        favourite: bool,
    ) -> Result<(), StateError> {
        let mut next = self.index.clone();
        let e = next
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("unknown preset")?;
        e.name = name;
        e.tags = tags;
        e.bank = bank;
        e.favourite = favourite;
        self.commit(next)
    }
    pub fn remove(&mut self, id: &str) -> Result<(), StateError> {
        self.entry(id)?;
        let mut next = self.index.clone();
        next.entries.retain(|e| e.id != id);
        self.commit(next)
    }
    /// Case-insensitive AND search across name/bank/tags, stable name/ID ordering.
    pub fn search(
        &self,
        query: &str,
        bank: Option<&str>,
        favourites_only: bool,
    ) -> Vec<&PresetEntry> {
        let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut entries: Vec<_> = self
            .index
            .entries
            .iter()
            .filter(|e| {
                (!favourites_only || e.favourite) && bank.is_none_or(|b| e.bank == b) && {
                    let haystack =
                        format!("{} {} {}", e.name, e.bank, e.tags.join(" ")).to_lowercase();
                    words.iter().all(|w| haystack.contains(w))
                }
            })
            .collect();
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        entries
    }
    pub fn project(&self, id: &str) -> Result<(ProjectState, PathBuf), StateError> {
        let e = self.entry(id)?;
        let bytes = read(&self.root.join(&e.project_file), 4 * 1024 * 1024)?;
        let digest = e
            .project_file
            .strip_prefix("presets/")
            .unwrap()
            .strip_suffix(".json")
            .unwrap();
        if hash(&bytes) != digest {
            return Err("preset document checksum mismatch".into());
        }
        Ok((
            ProjectState::from_json(std::str::from_utf8(&bytes)?)?,
            e.asset_root.clone(),
        ))
    }
    pub fn missing_assets(&self, id: &str) -> Result<Vec<(String, PathBuf)>, StateError> {
        let (s, root) = self.project(id)?;
        Ok(s.assets
            .iter()
            .filter(|a| s.resolve_asset(&root, a).is_err())
            .map(|a| (a.id.clone(), root.join(&a.path)))
            .collect())
    }
    /// Rebind root/relative paths only after every referenced asset resolves safely.
    /// Catalog ID/metadata remain stable and the previous immutable document survives.
    pub fn relocate(
        &mut self,
        id: &str,
        root: &Path,
        paths: &BTreeMap<String, String>,
    ) -> Result<(), StateError> {
        let (mut s, _) = self.project(id)?;
        if paths.keys().any(|id| !s.assets.iter().any(|a| &a.id == id)) {
            return Err("unknown relocation asset ID".into());
        }
        for a in &mut s.assets {
            if let Some(p) = paths.get(&a.id) {
                a.path = p.clone();
            }
        }
        s.validate(&s.product_id)?;
        for a in &s.assets {
            s.resolve_asset(root, a)?;
        }
        let root = root.canonicalize()?;
        let filename = self.store_project(&s)?;
        let mut next = self.index.clone();
        let e = next
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("unknown preset")?;
        e.project_file = filename;
        e.asset_root = root;
        self.commit(next)
    }
    pub fn export_preset(&self, id: &str, destination: &Path) -> Result<(), StateError> {
        let (s, root) = self.project(id)?;
        crate::project_storage::pack_bundle(&s, &root, destination)
    }
}

#[cfg(feature = "ui")]
pub mod browser {
    use super::*;
    use crate::ui::egui;
    #[derive(Clone, Debug)]
    pub enum BrowserAction {
        Store,
        Load(String),
        Preview(String),
        Audition,
        ToggleFavourite(String),
        Remove(String),
    }
    #[derive(Default)]
    pub struct PresetBrowser {
        pub query: String,
        pub bank: Option<String>,
        pub favourites_only: bool,
        pub selected: Option<String>,
    }
    impl PresetBrowser {
        /// Draw only; caller handles all allocating/file/audio actions outside this widget.
        pub fn show(
            &mut self,
            ui: &mut egui::Ui,
            catalog: &PresetCatalog,
        ) -> Option<BrowserAction> {
            let mut action = None;
            ui.horizontal_wrapped(|ui| {
                ui.label("Search");
                ui.text_edit_singleline(&mut self.query);
                ui.checkbox(&mut self.favourites_only, "Favourites only");
            });
            let banks: BTreeSet<_> = catalog.entries().iter().map(|e| e.bank.clone()).collect();
            egui::ComboBox::from_id_salt("preset-bank")
                .selected_text(self.bank.as_deref().unwrap_or("All banks"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.bank, None, "All banks");
                    for bank in banks {
                        ui.selectable_value(&mut self.bank, Some(bank.clone()), bank);
                    }
                });
            egui::ScrollArea::vertical()
                .id_salt("preset-browser-list")
                .max_height(180.0)
                .show(ui, |ui| {
                    for e in catalog.search(&self.query, self.bank.as_deref(), self.favourites_only)
                    {
                        if ui
                            .selectable_label(
                                self.selected.as_ref() == Some(&e.id),
                                format!(
                                    "{}{} · {} · {}",
                                    if e.favourite { "★ " } else { "" },
                                    e.name,
                                    e.bank,
                                    e.tags.join(", ")
                                ),
                            )
                            .clicked()
                        {
                            self.selected = Some(e.id.clone());
                        }
                    }
                });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Store current project").clicked() {
                    action = Some(BrowserAction::Store);
                }
                if let Some(id) = &self.selected {
                    for (label, a) in [
                        ("Load", BrowserAction::Load(id.clone())),
                        ("Render preview", BrowserAction::Preview(id.clone())),
                        ("Audition preview", BrowserAction::Audition),
                        ("Favourite", BrowserAction::ToggleFavourite(id.clone())),
                        ("Remove from index", BrowserAction::Remove(id.clone())),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(a);
                        }
                    }
                }
            });
            action
        }
    }
}

/// Allocated/decoded on control, then borrowed/interpolated without callback I/O.
pub struct PreviewPlayer {
    audio: Arc<crate::io::AudioFile>,
    position: f64,
    step: f64,
    prepared: bool,
}
impl PreviewPlayer {
    pub fn new(audio: Arc<crate::io::AudioFile>) -> Result<Self, Error> {
        if audio.sample_rate == 0
            || audio.channels == 0
            || audio.channels > 64
            || audio.samples.is_empty()
            || !audio.samples.len().is_multiple_of(audio.channels as usize)
            || audio.samples.iter().any(|x| !x.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self {
            audio,
            position: 0.0,
            step: 1.0,
            prepared: false,
        })
    }
}
impl AudioProcessor for PreviewPlayer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.prepared = false;
        s.validate()?;
        self.step = self.audio.sample_rate as f64 / s.sample_rate;
        self.position = 0.0;
        self.prepared = true;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        let source = self.audio.channels as usize;
        for f in b.samples_mut().chunks_exact_mut(ch) {
            if !self.prepared || self.position >= self.audio.frames() as f64 {
                f.fill(0.0);
                continue;
            }
            let i = self.position as usize;
            let j = (i + 1).min(self.audio.frames() - 1);
            let t = (self.position - i as f64) as f32;
            for (c, x) in f.iter_mut().enumerate() {
                let c = c.min(source - 1);
                let a = self.audio.samples[i * source + c];
                *x = a + (self.audio.samples[j * source + c] - a) * t;
            }
            self.position += self.step;
        }
    }
    fn reset(&mut self) {
        self.position = 0.0;
    }
}
