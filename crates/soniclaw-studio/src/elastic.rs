//! Worker-only transient detection and shared-decision multitrack stretching.
use crate::{
    project::{Project, ProjectError},
    tempo_cache::{self, Binding, Marker, Recipe},
    *,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detection {
    pub threshold: f32,
    pub floor_db: f32,
    pub gap_ms: f32,
}
impl Default for Detection {
    fn default() -> Self {
        Self {
            threshold: 4.,
            floor_db: -45.,
            gap_ms: 50.,
        }
    }
}
/// Peak-energy candidates in source frames; audition/refine before committing timing edits.
pub fn detect(path: &Path, s: Detection) -> Result<Vec<u64>, ProjectError> {
    if !s.threshold.is_finite()
        || !(1.1..=20.).contains(&s.threshold)
        || !s.floor_db.is_finite()
        || !(-80. ..=-6.).contains(&s.floor_db)
        || !s.gap_ms.is_finite()
        || !(10. ..=1000.).contains(&s.gap_ms)
    {
        return Err("invalid transient detection settings".into());
    }
    let rate = hound::WavReader::open(path)?.spec().sample_rate;
    let a = crate::media_tools::decode(path, rate)?;
    let floor = 10f32.powf(s.floor_db / 10.);
    let fast_step = 1. - (-1. / (rate as f32 * 0.0005)).exp();
    let slow_step = 1. - (-1. / (rate as f32 * 0.03)).exp();
    let gap = (rate as f32 * s.gap_ms / 1000.).round() as u64;
    let (mut fast, mut slow) = (0f32, 0f32);
    let mut result = vec![];
    let mut armed = true;
    let mut last = None;
    for (i, f) in a.samples.chunks_exact(a.channels as usize).enumerate() {
        let e = f.iter().map(|v| v * v).fold(0., f32::max);
        fast += fast_step * (e - fast);
        slow += slow_step * (e - slow);
        if fast < slow * 1.2 || fast < floor * 0.25 {
            armed = true;
        }
        if armed
            && fast > floor
            && fast > slow * s.threshold
            && last.is_none_or(|last| i as u64 >= last + gap)
        {
            if result.len() >= 254 {
                return Err("over 254 detected onsets; narrow or raise threshold".into());
            }
            result.push(i as u64);
            last = Some(i as u64);
            armed = false;
        }
    }
    Ok(result)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub source: String,
    pub track: String,
    pub region: String,
    pub asset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub id: String,
    pub reference: usize,
    pub members: Vec<Member>,
    pub recipe: Recipe,
}
impl Group {
    pub fn validate(&self, p: &Project) -> Result<(), ProjectError> {
        if !valid_id(&self.id)
            || !(1..=16).contains(&self.members.len())
            || self.reference >= self.members.len()
        {
            return Err("invalid elastic group/reference".into());
        }
        let mut sources = BTreeSet::new();
        let mut assets = BTreeSet::new();
        let mut regions = BTreeSet::new();
        let mut shape = None;
        let mut width = 0u64;
        for m in &self.members {
            if !valid_id(&m.source)
                || !valid_id(&m.asset)
                || !valid_id(&m.region)
                || m.asset == m.source
                || !sources.insert(&m.source)
                || !assets.insert(&m.asset)
                || !regions.insert(&m.region)
                || p.warps.contains_key(&m.source)
                || p.freezes.contains_key(&m.track)
                || !p.document.tracks.iter().any(|t| t.id == m.track)
            {
                return Err("invalid, duplicate or frozen elastic member".into());
            }
            let r =
                hound::WavReader::open(p.disk.get(&m.source).ok_or("elastic original missing")?)?;
            if !(1..=2).contains(&r.spec().channels)
                || !(8000..=192000).contains(&r.spec().sample_rate)
            {
                return Err("elastic sources require mono/stereo WAV".into());
            }
            width += r.spec().channels as u64;
            let current = (r.spec().sample_rate, r.duration());
            if shape.is_some_and(|s| s != current) {
                return Err("phase-linked originals must share sample rate and frame count".into());
            }
            shape = Some(current);
            self.recipe.validate(
                r.duration() as u64,
                &p.document.timeline,
                p.document.sample_rate,
            )?;
            if p.disk.contains_key(&m.asset) && !p.warps.contains_key(&m.asset) {
                return Err("elastic asset collides with original media".into());
            }
            if let Some(id) = &m.replace {
                let t = p
                    .document
                    .tracks
                    .iter()
                    .find(|t| t.id == m.track)
                    .ok_or("track missing")?;
                let c = t
                    .streamed
                    .iter()
                    .find(|c| &c.id == id && c.asset == m.source)
                    .ok_or("original region missing")?;
                if c.looping || c.source_offset != 0 || c.source_length != r.duration() as u64 {
                    return Err("replacement requires a full-file unlooped original region".into());
                }
            }
        }
        let (_, frames) = shape.ok_or("elastic sources missing")?;
        let map = TempoMap::compile(&p.document.timeline)?;
        let a = map.sample_at(self.recipe.markers[0].beat, p.document.sample_rate as f64)?;
        let b = map.sample_at(
            self.recipe.markers.last().unwrap().beat,
            p.document.sample_rate as f64,
        )?;
        if (frames as u64)
            .max(b - a)
            .checked_mul(width * 4)
            .is_none_or(|n| n > 2 * 1024 * 1024 * 1024)
        {
            return Err("linked working WAV exceeds 2 GiB".into());
        }
        Ok(())
    }
    /// Snap onset anchors to an absolute beat grid. Fixed endpoints remain; colliding snaps are omitted.
    pub fn quantize(
        &mut self,
        p: &Project,
        onsets: &[u64],
        grid: f64,
    ) -> Result<usize, ProjectError> {
        self.validate(p)?;
        if !grid.is_finite()
            || !(1. / 64. ..=4.).contains(&grid)
            || onsets.len() > 254
            || onsets.windows(2).any(|w| w[0] >= w[1])
        {
            return Err("invalid onset grid/list".into());
        }
        let r = hound::WavReader::open(&p.disk[&self.members[self.reference].source])?;
        let map = TempoMap::compile(&p.document.timeline)?;
        let first = self
            .recipe
            .markers
            .first()
            .ok_or("warp start missing")?
            .clone();
        let last = self
            .recipe
            .markers
            .last()
            .ok_or("warp end missing")?
            .clone();
        let origin = map.sample_at(first.beat, p.document.sample_rate as f64)?;
        let mut markers = vec![first.clone()];
        for &source in onsets {
            if source <= first.source_frame || source >= last.source_frame {
                continue;
            }
            let sample = origin
                + ((source - first.source_frame) as f64 * p.document.sample_rate as f64
                    / r.spec().sample_rate as f64)
                    .round() as u64;
            let beat =
                (map.beat_at_sample(sample, p.document.sample_rate as f64)? / grid).round() * grid;
            if beat > markers.last().unwrap().beat && beat < last.beat {
                markers.push(Marker {
                    source_frame: source,
                    beat,
                });
            }
        }
        markers.push(last);
        let count = markers.len() - 2;
        let mut next = self.clone();
        next.recipe.markers = markers;
        next.validate(p)?;
        *self = next;
        Ok(count)
    }
    /// Own a new output folder. Failure removes only that folder; the project remains unchanged.
    pub fn build(
        &self,
        p: &Project,
        directory: &Path,
        cancel: &AtomicBool,
    ) -> Result<Ready, ProjectError> {
        self.validate(p)?;
        p.validate()?;
        std::fs::create_dir(directory)?;
        let result = (|| -> Result<Ready, ProjectError> {
            let mut readers = vec![];
            let mut channels = 0usize;
            let mut reference = 0usize;
            let mut source_hashes = vec![];
            for (i, m) in self.members.iter().enumerate() {
                let path = &p.disk[&m.source];
                source_hashes.push(tempo_cache::hash(path)?);
                let reader = hound::WavReader::open(path)?;
                if i == self.reference {
                    reference = channels;
                }
                channels += reader.spec().channels as usize;
                readers.push(reader);
            }
            let rate = readers[0].spec().sample_rate;
            let frames = readers[0].duration();
            let packed = directory.join("linked-source.wav");
            let mut writer = hound::WavWriter::create(
                &packed,
                hound::WavSpec {
                    channels: channels as u16,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )?;
            // One analysis stream selects every WSOLA grain offset/fade across all microphone channels.
            for frame in 0..frames {
                if frame.is_multiple_of(4096) && cancel.load(Ordering::Acquire) {
                    return Err("elastic preparation cancelled".into());
                }
                for reader in &mut readers {
                    let spec = reader.spec();
                    for _ in 0..spec.channels {
                        let v = if spec.sample_format == hound::SampleFormat::Float {
                            reader
                                .samples::<f32>()
                                .next()
                                .ok_or("short elastic source")??
                        } else {
                            reader
                                .samples::<i32>()
                                .next()
                                .ok_or("short elastic source")?? as f32
                                / 2f32.powi(spec.bits_per_sample as i32 - 1)
                        };
                        writer.write_sample(v)?;
                    }
                }
            }
            writer.finalize()?;
            let wide = tempo_cache::prepare_linked(
                &packed,
                &self.recipe,
                &p.document.timeline,
                p.document.sample_rate,
                &directory.join("linked-render"),
                cancel,
                Some(reference),
            )?;
            let mut rendered = hound::WavReader::open(directory.join("linked-render/audio.wav"))?;
            let mut writers = vec![];
            for (i, reader) in readers.iter().enumerate() {
                let folder = directory.join(format!("member-{i}"));
                std::fs::create_dir(&folder)?;
                writers.push(hound::WavWriter::create(
                    folder.join("audio.wav"),
                    hound::WavSpec {
                        channels: reader.spec().channels,
                        sample_rate: p.document.sample_rate,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    },
                )?);
            }
            let mut samples = rendered.samples::<f32>();
            for frame in 0..wide.frames {
                if frame.is_multiple_of(4096) && cancel.load(Ordering::Acquire) {
                    return Err("elastic preparation cancelled".into());
                }
                for w in &mut writers {
                    for _ in 0..w.spec().channels {
                        w.write_sample(samples.next().ok_or("short linked render")??)?;
                    }
                }
            }
            for w in writers {
                w.finalize()?;
            }
            let mut outputs = vec![];
            for (i, m) in self.members.iter().enumerate() {
                let wav = directory.join(format!("member-{i}/audio.wav"));
                if tempo_cache::hash(&p.disk[&m.source])? != source_hashes[i] {
                    return Err("elastic original changed during preparation".into());
                }
                let binding = Binding {
                    source: m.source.clone(),
                    source_sha256: source_hashes[i].clone(),
                    wav_sha256: tempo_cache::hash(&wav)?,
                    ..wide.clone()
                };
                binding.verify(
                    &p.disk[&m.source],
                    &wav,
                    &p.document.timeline,
                    p.document.sample_rate,
                )?;
                std::fs::write(
                    directory.join(format!("member-{i}/warp.json")),
                    serde_json::to_vec_pretty(&binding)?,
                )?;
                outputs.push((binding, wav));
            }
            std::fs::write(
                directory.join("group.json"),
                serde_json::to_vec_pretty(self)?,
            )?;
            std::fs::remove_file(packed)?;
            std::fs::remove_dir_all(directory.join("linked-render"))?;
            Ok(Ready {
                baseline: p.clone(),
                group: self.clone(),
                outputs,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(directory);
        }
        result
    }
}
pub struct Ready {
    baseline: Project,
    pub group: Group,
    outputs: Vec<(Binding, PathBuf)>,
}
impl Ready {
    pub fn apply(self, p: &mut Project) -> Result<(), ProjectError> {
        if !crate::history::equivalent(&self.baseline, p)? {
            return Err("project changed during elastic render; rebuild".into());
        }
        let mut next = p.clone();
        next.layout.elastic_groups.retain(|g| g.id != self.group.id);
        for (m, (binding, wav)) in self.group.members.iter().zip(self.outputs) {
            next.apply_warp(&m.track, &m.region, &m.asset, binding, &wav)?;
            if let Some(id) = &m.replace {
                if let Some(c) = next
                    .document
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == m.track)
                    .and_then(|t| t.streamed.iter_mut().find(|c| &c.id == id))
                {
                    c.enabled = false;
                }
            }
        }
        next.layout.elastic_groups.push(self.group);
        next.validate()?;
        *p = next;
        Ok(())
    }
}
pub fn validate_saved(p: &Project) -> Result<(), ProjectError> {
    if p.layout.elastic_groups.len() > 16 {
        return Err("too many elastic groups".into());
    }
    let mut ids = BTreeSet::new();
    let mut assets = BTreeSet::new();
    for g in &p.layout.elastic_groups {
        g.validate(p)?;
        if !ids.insert(&g.id) {
            return Err("elastic group identity collision".into());
        }
        for m in &g.members {
            let b = p.warps.get(&m.asset).ok_or("elastic group cache missing")?;
            if !assets.insert(&m.asset)
                || b.source != m.source
                || b.recipe != g.recipe
                || !p.document.tracks.iter().any(|t| {
                    t.id == m.track
                        && t.streamed
                            .iter()
                            .any(|c| c.id == m.region && c.asset == m.asset)
                })
            {
                return Err(
                    "elastic group binding mismatch; dissolve group before independent edits"
                        .into(),
                );
            }
        }
    }
    Ok(())
}
/// Drop shared-edit enforcement while preserving rendered audio, recipes and original resources.
pub fn dissolve(p: &mut Project, id: &str) -> Result<(), ProjectError> {
    let mut next = p.clone();
    let n = next.layout.elastic_groups.len();
    next.layout.elastic_groups.retain(|g| g.id != id);
    if next.layout.elastic_groups.len() == n {
        return Err("elastic group missing".into());
    }
    next.validate()?;
    *p = next;
    Ok(())
}
