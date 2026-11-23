//! Portable unified-session folders. All filesystem work belongs on a control/worker thread.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
fn is_zero(n: &usize) -> bool {
    *n == 0
}
pub type ProjectError = Box<dyn std::error::Error + Send + Sync>;
const MANIFEST: usize = 16 * 1024 * 1024;
const DISK_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    id: String,
    file: String,
    sha256: String,
    memory: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    native_states: Vec<NativeResource>,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    warps: BTreeMap<String, tempo_cache::Binding>,
    #[serde(default)]
    comps: BTreeMap<String, Comp>,
    schema: u32,
    name: String,
    document: Document,
    resources: Vec<Resource>,
    freezes: Vec<String>,
    takes: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    midi_takes: usize,
    #[serde(default, skip_serializing_if = "crate::workspace::Layout::is_empty")]
    layout: crate::workspace::Layout,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comp {
    pub take: usize,
    pub stem: String,
    pub recipe: soniclaw_record::comp::Recipe,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeResource {
    id: String,
    file: String,
    metadata: crate::native_state::Metadata,
}
/// Combined memory/disk editing draft. Source aliases are stable and opaque to the UI.
pub struct ArrangementDraft {
    pub document: soniclaw_clips::Document,
    sources: BTreeMap<String, (bool, String)>,
}
/// Application data and resources; an explicit Factory is still required to instantiate processors.
#[derive(Clone)]
pub struct Project {
    pub native_states: crate::native_state::States,
    pub layout: crate::workspace::Layout,
    pub project_id: String,
    pub warps: BTreeMap<String, tempo_cache::Binding>,
    pub name: String,
    pub document: Document,
    pub assets: Assets,
    pub disk: DiskAssets,
    pub freezes: BTreeMap<String, freeze::Frozen>,
    pub takes: Vec<PathBuf>,
    pub midi_takes: Vec<PathBuf>,
    pub comps: BTreeMap<String, Comp>,
}
/// Result of a verified load; no native processor is instantiated.
pub struct OpenReport {
    pub project: Project,
    pub source_schema: u32,
    pub migrations: Vec<String>,
}
static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);
pub(crate) fn new_identity() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let bytes = format!(
        "{now}:{}:{}",
        std::process::id(),
        ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    format!("project.{:x}", Sha256::digest(bytes))[..56].to_owned()
}
fn digest(path: &Path) -> Result<String, ProjectError> {
    let mut f = std::fs::File::open(path)?;
    let mut bytes = [0; 65536];
    let mut h = Sha256::new();
    loop {
        let n = f.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        h.update(&bytes[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn copy(source: &Path, target: &Path, total: &mut u64) -> Result<(), ProjectError> {
    *total = total
        .checked_add(std::fs::metadata(source)?.len())
        .ok_or("resource size overflow")?;
    if *total > DISK_LIMIT {
        return Err("project resources exceed two GiB".into());
    }
    std::fs::copy(source, target)?;
    Ok(())
}
impl Project {
    pub fn new(
        name: String,
        document: Document,
        assets: Assets,
        disk: DiskAssets,
    ) -> Result<Self, ProjectError> {
        let p = Self {
            native_states: Default::default(),
            layout: Default::default(),
            project_id: new_identity(),
            name,
            document,
            assets,
            disk,
            freezes: BTreeMap::new(),
            takes: vec![],
            midi_takes: vec![],
            comps: BTreeMap::new(),
            warps: BTreeMap::new(),
        };
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<(), ProjectError> {
        self.document.validate()?;
        crate::native_state::validate(self)?;
        self.layout.validate(&self.document)?;
        crate::elastic::validate_saved(self)?;
        crate::takes::validate(self)?;
        if !valid_id(&self.project_id)
            || self.name.len() > 256
            || self.assets.len() + self.disk.len() > 1024
            || self.freezes.len() > 32
            || self.takes.len() > 256
            || self.midi_takes.len() > 256
            || self.comps.len() > 32
            || self.warps.len() > 32
        {
            return Err("project limits exceeded".into());
        }
        for (id, binding) in &self.warps {
            if !valid_id(id) {
                return Err("invalid tempo cache identity".into());
            }
            binding.verify(
                self.disk
                    .get(&binding.source)
                    .ok_or("missing warp source")?,
                self.disk.get(id).ok_or("missing warp cache")?,
                &self.document.timeline,
                self.document.sample_rate,
            )?;
        }
        let mut count = 0usize;
        for (id, a) in &self.assets {
            if !valid_id(id)
                || !(1..=2).contains(&a.channels)
                || a.sample_rate != self.document.sample_rate
                || a.samples.len() % a.channels as usize != 0
                || a.samples.iter().any(|s| !s.is_finite())
            {
                return Err("invalid memory asset".into());
            }
            count = count
                .checked_add(a.samples.len())
                .ok_or("asset size overflow")?;
        }
        if count > 64 * 1024 * 1024 {
            return Err("decoded assets exceed 64 Mi samples".into());
        }
        for id in self.disk.keys() {
            if !valid_id(id) {
                return Err("invalid disk identity".into());
            }
        }
        for (id, comp) in &self.comps {
            if !valid_id(id)
                || comp.take >= self.takes.len()
                || !self.document.tracks.iter().any(|t| &t.id == id)
            {
                return Err("invalid comp track/take reference".into());
            }
            let take = soniclaw_record::multitrack::Take::read(&self.takes[comp.take])?;
            comp.recipe.regions(&take, &comp.stem)?;
        }
        Ok(())
    }
    /// Exclusively creates a new portable folder. On failure removes only the new folder.
    pub fn save_new(&self, directory: &Path) -> Result<(), ProjectError> {
        self.validate()?;
        std::fs::create_dir(directory)?;
        let result = self.write(directory);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(directory);
        }
        result
    }
    fn write(&self, directory: &Path) -> Result<(), ProjectError> {
        std::fs::create_dir(directory.join("assets"))?;
        let mut resources = Vec::new();
        let mut total = 0;
        for (id, a) in &self.assets {
            let file = format!("assets/{}.wav", resources.len());
            let mut out = hound::WavWriter::create(
                directory.join(&file),
                hound::WavSpec {
                    channels: a.channels,
                    sample_rate: a.sample_rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )?;
            for s in a.samples.iter() {
                out.write_sample(*s)?;
            }
            out.finalize()?;
            total += std::fs::metadata(directory.join(&file))?.len();
            if total > DISK_LIMIT {
                return Err("project size limit".into());
            }
            resources.push(Resource {
                id: id.clone(),
                file: file.clone(),
                sha256: digest(&directory.join(file))?,
                memory: true,
            });
        }
        for (id, path) in &self.disk {
            let file = format!("assets/{}.wav", resources.len());
            copy(path, &directory.join(&file), &mut total)?;
            resources.push(Resource {
                id: id.clone(),
                file: file.clone(),
                sha256: digest(&directory.join(file))?,
                memory: false,
            });
        }
        std::fs::create_dir(directory.join("freezes"))?;
        let mut freezes = Vec::new();
        for (track, frozen) in &self.freezes {
            if track != &frozen.track {
                return Err("freeze track mismatch".into());
            }
            frozen.verify()?;
            let folder = directory.join("freezes").join(freezes.len().to_string());
            std::fs::create_dir(&folder)?;
            copy(
                &frozen.directory.join("stem.wav"),
                &folder.join("stem.wav"),
                &mut total,
            )?;
            let mut saved = frozen.clone();
            saved.directory = PathBuf::from(".");
            std::fs::write(
                folder.join("freeze.json"),
                serde_json::to_vec_pretty(&saved)?,
            )?;
            freezes.push(track.clone());
        }
        std::fs::create_dir(directory.join("takes"))?;
        for (index, path) in self.takes.iter().enumerate() {
            let take = soniclaw_record::multitrack::Take::read(path)?;
            let folder = directory.join("takes").join(index.to_string());
            std::fs::create_dir(&folder)?;
            for track in &take.tracks {
                let source = path.join(&track.file).canonicalize()?;
                let shared = resources.iter().find(|r| {
                    !r.memory
                        && self
                            .disk
                            .get(&r.id)
                            .and_then(|p| p.canonicalize().ok())
                            .as_ref()
                            == Some(&source)
                });
                if let Some(resource) = shared {
                    if std::fs::hard_link(directory.join(&resource.file), folder.join(&track.file))
                        .is_err()
                    {
                        copy(&source, &folder.join(&track.file), &mut total)?;
                    }
                } else {
                    copy(&source, &folder.join(&track.file), &mut total)?;
                }
            }
            for name in ["recording-schedule.json", "input-quality.json"] {
                let metadata = path.join(name);
                if metadata.exists() {
                    if std::fs::metadata(&metadata)?.len() > 32768 {
                        return Err("recording metadata too large".into());
                    }
                    copy(&metadata, &folder.join(name), &mut total)?;
                }
            }
            copy(
                &path.join("take.json"),
                &folder.join("take.json"),
                &mut total,
            )?;
        }
        std::fs::create_dir(directory.join("midi-takes"))?;
        for (index, path) in self.midi_takes.iter().enumerate() {
            let data = read_midi(path)?;
            let folder = directory.join("midi-takes").join(index.to_string());
            std::fs::create_dir(&folder)?;
            std::fs::write(folder.join("midi-take.json"), data.to_json()?)?;
            let smf = path.join("performance.mid");
            if smf.exists() {
                if std::fs::metadata(&smf)?.len() > 16 * 1024 * 1024 {
                    return Err("SMF too large".into());
                }
                soniclaw_midi::file::MidiClip::read(&std::fs::read(&smf)?)?;
                copy(&smf, &folder.join("performance.mid"), &mut total)?;
            }
        }
        let mut native_states = Vec::new();
        std::fs::create_dir(directory.join("native-states"))?;
        for (id, blob) in &self.native_states {
            let file = format!("native-states/{}.bin", native_states.len());
            total += blob.bytes().len() as u64;
            if total > DISK_LIMIT {
                return Err("project size limit".into());
            }
            let mut out = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(directory.join(&file))?;
            out.write_all(blob.bytes())?;
            out.sync_all()?;
            native_states.push(NativeResource {
                id: id.clone(),
                file,
                metadata: blob.metadata().clone(),
            });
        }
        let manifest = Manifest {
            native_states,
            project_id: Some(self.project_id.clone()),
            schema: 3,
            layout: self.layout.clone(),
            name: self.name.clone(),
            document: self.document.clone(),
            resources,
            freezes,
            takes: self.takes.len(),
            midi_takes: self.midi_takes.len(),
            comps: self.comps.clone(),
            warps: self.warps.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        if bytes.len() > MANIFEST {
            return Err("project manifest too large".into());
        }
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("project.json"))?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        Ok(())
    }
    /// Verify every resource before returning a candidate. Does not load native plug-in code.
    pub fn open(directory: &Path) -> Result<Self, ProjectError> {
        Ok(Self::open_with_report(directory)?.project)
    }
    pub fn open_with_report(directory: &Path) -> Result<OpenReport, ProjectError> {
        let root = crate::sessions::current_directory(directory)?;
        let mut bytes = Vec::new();
        let manifest_path = root.join("project.json").canonicalize()?;
        if !manifest_path.starts_with(&root) {
            return Err("project manifest escapes project folder".into());
        }
        std::fs::File::open(manifest_path)?
            .take((MANIFEST + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MANIFEST {
            return Err("project manifest too large".into());
        }
        let m: Manifest = serde_json::from_slice(&bytes)?;
        if !matches!(m.schema, 1..=3)
            || m.resources.len() > 1024
            || m.freezes.len() > 32
            || m.takes > 256
            || m.midi_takes > 256
        {
            return Err("invalid project manifest".into());
        }
        let source_schema = m.schema;
        let project_id = match (&m.project_id, m.schema) {
            (Some(id), _) if valid_id(id) => id.clone(),
            (None, 1) => {
                format!("legacy.{:x}", Sha256::digest(serde_json::to_vec(&m)?))[..55].to_owned()
            }
            _ => return Err("invalid/missing project identity".into()),
        };
        let mut assets = Assets::new();
        let mut disk = DiskAssets::new();
        let mut seen = BTreeSet::new();
        let mut total = 0u64;
        let mut samples = 0usize;
        for (index, r) in m.resources.iter().enumerate() {
            if !valid_id(&r.id)
                || r.file != format!("assets/{index}.wav")
                || !seen.insert((r.memory, &r.id))
            {
                return Err("invalid resource identity/path".into());
            }
            let path = root.join(&r.file).canonicalize()?;
            if !path.starts_with(&root) {
                return Err("resource escapes project".into());
            }
            total = total
                .checked_add(std::fs::metadata(&path)?.len())
                .ok_or("resource size overflow")?;
            if total > DISK_LIMIT || digest(&path)? != r.sha256 {
                return Err("resource missing/corrupt/oversized".into());
            }
            if r.memory {
                let reader = hound::WavReader::open(&path)?;
                let spec = reader.spec();
                if spec.sample_format != hound::SampleFormat::Float
                    || spec.bits_per_sample != 32
                    || !(1..=2).contains(&spec.channels)
                {
                    return Err("memory resource must be float mono/stereo".into());
                }
                samples = samples
                    .checked_add(reader.len() as usize)
                    .ok_or("sample size overflow")?;
                if samples > 64 * 1024 * 1024 {
                    return Err("decoded asset budget exceeded".into());
                }
                let pcm = reader
                    .into_samples::<f32>()
                    .collect::<Result<Vec<_>, _>>()?;
                assets.insert(
                    r.id.clone(),
                    soniclaw_clips::AudioAsset {
                        sample_rate: spec.sample_rate,
                        channels: spec.channels,
                        samples: Arc::from(pcm),
                    },
                );
            } else {
                disk.insert(r.id.clone(), path);
            }
        }
        let mut freezes = BTreeMap::new();
        for (index, track) in m.freezes.iter().enumerate() {
            let f = freeze::Frozen::read(&root.join("freezes").join(index.to_string()))?;
            if &f.track != track || freezes.insert(track.clone(), f).is_some() {
                return Err("freeze identity mismatch".into());
            }
        }
        let mut takes = Vec::new();
        for index in 0..m.takes {
            let path = root.join("takes").join(index.to_string());
            soniclaw_record::multitrack::Take::read(&path)?;
            takes.push(path);
        }
        let mut midi_takes = vec![];
        for index in 0..m.midi_takes {
            let path = root.join("midi-takes").join(index.to_string());
            if !path.canonicalize()?.starts_with(&root) {
                return Err("MIDI take escapes project".into());
            }
            read_midi(&path)?;
            midi_takes.push(path);
        }
        let mut native_states = crate::native_state::States::new();
        let native_totals =
            m.native_states
                .iter()
                .try_fold((0usize, 0usize), |(bytes, parameters), r| {
                    Some((
                        bytes.checked_add(r.metadata.bytes)?,
                        parameters.checked_add(r.metadata.parameters.len())?,
                    ))
                });
        if native_totals
            .is_none_or(|(bytes, parameters)| bytes > 128 * 1024 * 1024 || parameters > 65536)
        {
            return Err("native snapshot aggregate limits exceeded".into());
        }
        if m.native_states.len() > 256 {
            return Err("too many native snapshots".into());
        }
        for (i, r) in m.native_states.iter().enumerate() {
            if r.file != format!("native-states/{i}.bin")
                || !valid_id(&r.id)
                || native_states.contains_key(&r.id)
            {
                return Err("invalid native resource path or ID".into());
            }
            let path = root.join(&r.file).canonicalize()?;
            if !path.starts_with(&root) {
                return Err("native resource escapes project".into());
            }
            let size = std::fs::metadata(&path)?.len();
            total = total.checked_add(size).ok_or("project size overflow")?;
            if size > crate::native_state::MAX_BYTES as u64 || total > DISK_LIMIT {
                return Err("oversized native resource".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take((crate::native_state::MAX_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            native_states.insert(
                r.id.clone(),
                crate::native_state::Blob::from_stored(r.metadata.clone(), bytes)?,
            );
        }
        let project = Self {
            native_states,
            layout: m.layout,
            project_id,
            name: m.name,
            document: m.document,
            assets,
            disk,
            freezes,
            takes,
            midi_takes,
            comps: m.comps,
            warps: m.warps,
        };
        project.validate()?;
        Ok(OpenReport {
            project,
            source_schema,
            migrations: if source_schema < 3 {
                vec![format!("Project schema {source_schema} migrated to 3; stable identity and track layout supported")]
            } else {
                vec![]
            },
        })
    }
    /// Add/update an audio-only comp track. Original source tracks and recordings are retained.
    /// Source stems are already post-insert; no original insert chain is applied twice.
    pub fn apply_comp(&mut self, id: &str, comp: Comp) -> Result<(), ProjectError> {
        if !valid_id(id) || comp.take >= self.takes.len() {
            return Err("invalid comp destination".into());
        }
        let directory = &self.takes[comp.take];
        let take = soniclaw_record::multitrack::Take::read(directory)?;
        let regions = comp.recipe.regions(&take, &comp.stem)?;
        if take.sample_rate != self.document.sample_rate
            || take
                .tracks
                .iter()
                .find(|t| t.id == comp.stem)
                .is_none_or(|t| t.channels > 2)
        {
            return Err("comp stem must be project-rate mono/stereo".into());
        }
        if self.document.tracks.iter().any(|t| t.id == id) && !self.comps.contains_key(id) {
            return Err("destination is an existing non-comp track".into());
        }
        let asset = format!("comp.{id}");
        if !valid_id(&asset) {
            return Err("comp asset ID too long".into());
        }
        let clips = regions
            .iter()
            .enumerate()
            .map(|(index, r)| soniclaw_stream::clips::DiskClip {
                edits: None,
                id: format!(
                    "comp.{}.{index}",
                    &format!("{:x}", Sha256::digest(id.as_bytes()))[..24]
                ),
                asset: asset.clone(),
                start_frame: r.timeline_frame,
                length_frames: r.frames,
                source_offset: r.source_offset,
                source_length: r.frames,
                looping: false,
                gain: 1.,
                fade_in: r.fade_in,
                fade_out: r.fade_out,
                enabled: true,
            })
            .collect();
        let mut next = self.document.clone();
        let existing_inserts = self
            .document
            .tracks
            .iter()
            .find(|t| t.id == id)
            .filter(|_| self.comps.contains_key(id))
            .map_or_else(Vec::new, |t| t.inserts.clone());
        let track = Track {
            id: id.into(),
            audio: vec![],
            midi: vec![],
            instrument: None,
            inserts: existing_inserts,
            input_gain: 0.,
            midi_input: false,
            streamed: clips,
        };
        if let Some(index) = next.tracks.iter().position(|t| t.id == id) {
            next.tracks[index] = track;
        } else {
            next.tracks.push(track);
            next.mixer.strips.push(soniclaw_mixer::Strip {
                id: id.into(),
                name: id.into(),
                gain: 1.,
                pan: 0.,
                mute: false,
                solo: false,
                group: None,
                sends: vec![0.; next.mixer.taps.len()],
            });
        }
        next.validate()?;
        let source = directory.join(&regions.first().ok_or("empty comp")?.file);
        let mut candidate = self.clone();
        candidate.document = next;
        candidate.disk.insert(asset, source);
        candidate.comps.insert(id.into(), comp);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Unified nondestructive view of memory and disk regions without decoding.
    pub fn edit_view(&self) -> Result<ArrangementDraft, ProjectError> {
        let mut sources = BTreeMap::new();
        let mut reverse = BTreeMap::new();
        for (memory, id) in self
            .assets
            .keys()
            .map(|id| (true, id))
            .chain(self.disk.keys().map(|id| (false, id)))
        {
            let hash = format!(
                "{:x}",
                Sha256::digest(format!("{memory}:{}:{id}", id.len()).as_bytes())
            );
            let alias = format!("source.{}", &hash[..32]);
            if let Some(old) = sources.insert(alias.clone(), (memory, id.clone())) {
                if old != (memory, id.clone()) {
                    return Err("source alias collision".into());
                }
            }
            reverse.insert((memory, id.clone()), alias);
        }
        let mut audio = Vec::new();
        let mut midi = Vec::new();
        for (index, t) in self.document.tracks.iter().enumerate() {
            for c in &t.audio {
                let mut c = c.clone();
                c.track = index as u16;
                c.asset = reverse
                    .get(&(true, c.asset.clone()))
                    .ok_or("memory source missing")?
                    .clone();
                audio.push(c);
            }
            for c in &t.streamed {
                let mut c = c.as_audio(index as u16);
                c.asset = reverse
                    .get(&(false, c.asset.clone()))
                    .ok_or("disk source missing")?
                    .clone();
                audio.push(c);
            }
            for c in &t.midi {
                let mut c = c.clone();
                c.track = index as u16;
                midi.push(c);
            }
        }
        let document = soniclaw_clips::Document {
            schema: 1,
            sample_rate: self.document.sample_rate,
            channels: 2,
            timeline: self.document.timeline.clone(),
            audio,
            midi,
        };
        document.validate()?;
        Ok(ArrangementDraft { document, sources })
    }
    /// Commit an edited view with its original typed catalog. No file is changed.
    pub fn replace_edit_view(&mut self, view: ArrangementDraft) -> Result<(), ProjectError> {
        let d = view.document;
        d.validate()?;
        if d.sample_rate != self.document.sample_rate
            || d.channels != 2
            || d.audio
                .iter()
                .map(|c| c.track)
                .chain(d.midi.iter().map(|c| c.track))
                .any(|t| t as usize >= self.document.tracks.len())
        {
            return Err("editing draft format/track mismatch".into());
        }
        let mut next = self.document.clone();
        for t in &mut next.tracks {
            t.audio.clear();
            t.midi.clear();
            t.streamed.clear();
        }
        for mut c in d.audio {
            let (memory, id) = view
                .sources
                .get(&c.asset)
                .ok_or("audio source was removed; reopen the project")?;
            c.asset = id.clone();
            let index = c.track as usize;
            if *memory {
                if !self.assets.contains_key(id) {
                    return Err("memory source was removed".into());
                }
                next.tracks[index].audio.push(c);
            } else {
                if !self.disk.contains_key(id) {
                    return Err("disk source was removed".into());
                }
                next.tracks[index]
                    .streamed
                    .push(soniclaw_stream::clips::DiskClip::from_audio(c));
            }
        }
        for c in d.midi {
            next.tracks[c.track as usize].midi.push(c);
        }
        next.validate()?;
        let mut candidate = self.clone();
        candidate.document = next;
        crate::takes::reconcile(&mut candidate);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Combined clip draft in session track order; region IDs must be globally unique.
    pub fn arrangement(&self) -> Result<soniclaw_clips::Document, ProjectError> {
        let mut audio = Vec::new();
        let mut midi = Vec::new();
        for (index, t) in self.document.tracks.iter().enumerate() {
            for c in &t.audio {
                let mut c = c.clone();
                c.track = index as u16;
                audio.push(c);
            }
            for c in &t.midi {
                let mut c = c.clone();
                c.track = index as u16;
                midi.push(c);
            }
        }
        let d = soniclaw_clips::Document {
            schema: 1,
            sample_rate: self.document.sample_rate,
            channels: 2,
            timeline: self.document.timeline.clone(),
            audio,
            midi,
        };
        d.validate()?;
        Ok(d)
    }
    /// Commit a combined edit only after all referenced track tags and the full session validate.
    pub fn replace_arrangement(&mut self, d: soniclaw_clips::Document) -> Result<(), ProjectError> {
        d.validate()?;
        if d.sample_rate != self.document.sample_rate
            || d.channels != 2
            || d.audio
                .iter()
                .map(|c| c.track)
                .chain(d.midi.iter().map(|c| c.track))
                .any(|t| t as usize >= self.document.tracks.len())
        {
            return Err("arrangement format/track mismatch".into());
        }
        let mut next = self.document.clone();
        for t in &mut next.tracks {
            t.audio.clear();
            t.midi.clear();
        }
        for c in d.audio {
            next.tracks[c.track as usize].audio.push(c);
        }
        for c in d.midi {
            next.tracks[c.track as usize].midi.push(c);
        }
        next.validate()?;
        let mut candidate = self.clone();
        candidate.document = next;
        crate::takes::reconcile(&mut candidate);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Validated transactional clip draft replacement, preserving the rest of the session.
    pub fn replace_clips(
        &mut self,
        track: &str,
        clips: soniclaw_clips::Document,
    ) -> Result<(), ProjectError> {
        clips.validate()?;
        if clips.sample_rate != self.document.sample_rate || clips.channels != 2 {
            return Err("clip/session format mismatch".into());
        }
        let mut next = self.document.clone();
        let target = next
            .tracks
            .iter_mut()
            .find(|t| t.id == track)
            .ok_or("track missing")?;
        target.audio = clips.audio;
        target.midi = clips.midi;
        next.validate()?;
        let mut candidate = self.clone();
        candidate.document = next;
        crate::takes::reconcile(&mut candidate);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn clips(&self, track: &str) -> Result<soniclaw_clips::Document, ProjectError> {
        let t = self
            .document
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or("track missing")?;
        Ok(self.document.clip_document(t))
    }
    /// Validate a completed worker result against the current project, then commit.
    pub fn apply_frozen(&mut self, frozen: freeze::Frozen) -> Result<(), ProjectError> {
        if self.freezes.contains_key(&frozen.track) {
            return Err("track already frozen".into());
        }
        let (document, disk) = frozen.apply(&self.document, &self.assets, &self.disk)?;
        self.document = document;
        self.disk = disk;
        self.freezes.insert(frozen.track.clone(), frozen);
        Ok(())
    }
    /// Render and apply a cache. Failed rendering leaves the project unchanged.
    pub fn freeze(
        &mut self,
        track: &str,
        factory: &mut impl Factory,
        directory: &Path,
        start: u64,
        length: u64,
        tail: u64,
    ) -> Result<(), ProjectError> {
        if self.freezes.contains_key(track) {
            return Err("track already frozen".into());
        }
        let frozen = freeze::render(
            &self.document,
            track,
            &self.assets,
            &self.disk,
            factory,
            directory,
            start,
            length,
            tail,
        )?;
        let (document, disk) = frozen.apply(&self.document, &self.assets, &self.disk)?;
        self.document = document;
        self.disk = disk;
        self.freezes.insert(track.into(), frozen);
        Ok(())
    }
    pub fn unfreeze(&mut self, track: &str) -> Result<(), ProjectError> {
        let frozen = self.freezes.get(track).ok_or("track is not frozen")?;
        let mut restored = frozen.unfreeze(&self.document)?;
        crate::workspace::reindex(&mut restored);
        self.document = restored;
        self.disk.remove(&format!("freeze.{track}"));
        self.freezes.remove(track);
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Memory,
    Disk,
}
#[derive(Clone, Debug)]
pub struct ResourceIssue {
    pub id: String,
    pub kind: ResourceKind,
    pub file: String,
    pub reason: String,
}
fn inspect_manifest(root: &Path) -> Result<Manifest, ProjectError> {
    let mut bytes = vec![];
    let manifest_path = root.join("project.json").canonicalize()?;
    if !manifest_path.starts_with(root) {
        return Err("project manifest escapes project folder".into());
    }
    std::fs::File::open(manifest_path)?
        .take((MANIFEST + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MANIFEST {
        return Err("manifest too large".into());
    }
    let m: Manifest = serde_json::from_slice(&bytes)?;
    if !matches!(m.schema, 1..=3) || m.resources.len() > 1024 {
        return Err("unsupported project manifest".into());
    }
    for (i, r) in m.resources.iter().enumerate() {
        if !valid_id(&r.id) || r.file != format!("assets/{i}.wav") {
            return Err("invalid resource path/identity".into());
        }
    }
    Ok(m)
}
impl Project {
    pub fn resource_issues(directory: &Path) -> Result<Vec<ResourceIssue>, ProjectError> {
        let root = crate::sessions::current_directory(directory)?;
        let m = inspect_manifest(&root)?;
        let mut issues = vec![];
        for r in m.resources {
            let result = (|| -> Result<(), ProjectError> {
                let p = root.join(&r.file).canonicalize()?;
                if !p.starts_with(&root) || digest(&p)? != r.sha256 {
                    return Err("resource escapes project or checksum differs".into());
                }
                Ok(())
            })();
            if let Err(e) = result {
                issues.push(ResourceIssue {
                    id: r.id,
                    kind: if r.memory {
                        ResourceKind::Memory
                    } else {
                        ResourceKind::Disk
                    },
                    file: r.file,
                    reason: e.to_string(),
                });
            }
        }
        Ok(issues)
    }
    /// Fill one missing resource with byte-identical content; never overwrites an existing file.
    pub fn repair_missing(
        directory: &Path,
        id: &str,
        kind: ResourceKind,
        replacement: &Path,
    ) -> Result<(), ProjectError> {
        let root = crate::sessions::current_directory(directory)?;
        let m = inspect_manifest(&root)?;
        let r = m
            .resources
            .iter()
            .find(|r| r.id == id && r.memory == (kind == ResourceKind::Memory))
            .ok_or("unknown resource")?;
        if std::fs::metadata(replacement)?.len() > DISK_LIMIT || digest(replacement)? != r.sha256 {
            return Err("replacement content does not match original checksum".into());
        }
        let target = root.join(&r.file);
        let parent = target.parent().unwrap().canonicalize()?;
        if !parent.starts_with(&root) {
            return Err("resource parent escapes project".into());
        }
        let temporary = parent.join(format!(".repair-{}.tmp", new_identity()));
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let result = (|| -> Result<(), ProjectError> {
            let input = std::fs::File::open(replacement)?;
            let copied = std::io::copy(&mut input.take(DISK_LIMIT + 1), &mut output)?;
            if copied > DISK_LIMIT {
                return Err("replacement exceeds resource size limit".into());
            }
            output.sync_all()?;
            if digest(&temporary)? != r.sha256 {
                return Err("replacement changed while copying".into());
            }
            std::fs::hard_link(&temporary, &target)?;
            Ok(())
        })();
        drop(output);
        let _ = std::fs::remove_file(temporary);
        result
    }
}

fn read_midi(path: &Path) -> Result<crate::midi_recording::Take, ProjectError> {
    let mut bytes = String::new();
    std::fs::File::open(path.join("midi-take.json"))?
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut bytes)?;
    crate::midi_recording::Take::from_json(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_migration_and_exact_missing_resource_repair() {
        let root =
            std::env::temp_dir().join(format!("soniclaw-migration61-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let (d, a) = crate::tests::fixture();
        let p = Project::new("Legacy".into(), d, a, Default::default()).unwrap();
        let folder = root.join("project");
        p.save_new(&folder).unwrap();
        std::fs::copy(folder.join("assets/0.wav"), root.join("original.wav")).unwrap();
        std::fs::remove_file(folder.join("assets/0.wav")).unwrap();
        assert!(Project::open(&folder).is_err());
        let issues = Project::resource_issues(&folder).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, ResourceKind::Memory);
        Project::repair_missing(
            &folder,
            &issues[0].id,
            issues[0].kind,
            &root.join("original.wav"),
        )
        .unwrap();
        assert!(Project::repair_missing(
            &folder,
            &issues[0].id,
            issues[0].kind,
            &root.join("original.wav")
        )
        .is_err());
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("project.json")).unwrap()).unwrap();
        json["schema"] = serde_json::json!(1);
        json.as_object_mut().unwrap().remove("project_id");
        std::fs::write(
            folder.join("project.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        let loaded = Project::open_with_report(&folder).unwrap();
        assert_eq!(loaded.source_schema, 1);
        assert_eq!(loaded.migrations.len(), 1);
        assert_eq!(
            Project::open(&folder).unwrap().project_id,
            loaded.project.project_id
        );
        loaded.project.save_new(&root.join("upgraded")).unwrap();
        let migrated = Project::open_with_report(&root.join("upgraded")).unwrap();
        assert_eq!(migrated.source_schema, 3);
        assert!(migrated.migrations.is_empty());
        json["schema"] = serde_json::json!(999);
        std::fs::write(
            folder.join("project.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(Project::open(&folder).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn portable_roundtrip_and_corruption_leave_original_intact() {
        let (d, a) = crate::tests::fixture();
        let root = std::env::temp_dir().join(format!("soniclaw-project52-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = Project::new("Test".into(), d, a, DiskAssets::new()).unwrap();
        p.save_new(&root).unwrap();
        assert!(p.save_new(&root).is_err());
        let loaded = Project::open(&root).unwrap();
        assert_eq!(
            loaded.document.to_json().unwrap(),
            p.document.to_json().unwrap()
        );
        assert_eq!(
            loaded.assets["a"].samples.as_ref(),
            p.assets["a"].samples.as_ref()
        );
        std::fs::write(root.join("assets/0.wav"), b"bad").unwrap();
        assert!(Project::open(&root).is_err());
        assert_eq!(p.name, "Test");
        std::fs::remove_dir_all(root).unwrap();
    }
}
