//! Control/worker-only tempo-warp cache preparation. Playback uses ordinary disk regions.
use crate::{
    project::{Project, ProjectError},
    *,
};
use sha2::{Digest, Sha256};
use soniclaw_dsp::{
    resample::ResampleQuality,
    streaming::{StretchSettings, TimeStretcher},
};
use std::{path::Path, sync::atomic::AtomicBool};
const BLOCK: usize = 1024;
const READ: usize = BLOCK * 32 + 1028;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub source_frame: u64,
    pub beat: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Mode {
    PreservePitch,
    Repitch,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub markers: Vec<Marker>,
    pub mode: Mode,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub schema: u32,
    pub source: String,
    pub recipe: Recipe,
    pub source_sha256: String,
    pub timeline_sha256: String,
    pub wav_sha256: String,
    pub sample_rate: u32,
    pub start_frame: u64,
    pub frames: u64,
}
pub(crate) fn hash(path: &Path) -> Result<String, ProjectError> {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path)?;
    let mut b = [0; 65536];
    loop {
        let n = std::io::Read::read(&mut f, &mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn timeline_hash(t: &Timeline) -> Result<String, ProjectError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(t)?)))
}
impl Recipe {
    pub fn validate(&self, frames: u64, t: &Timeline, rate: u32) -> Result<(), ProjectError> {
        TempoMap::compile(t)?;
        if self.schema != 1
            || !(2..=256).contains(&self.markers.len())
            || !(8000..=192000).contains(&rate)
        {
            return Err("invalid warp recipe/rate".into());
        }
        for (i, m) in self.markers.iter().enumerate() {
            if !m.beat.is_finite()
                || !(0.0..=1_000_000.).contains(&m.beat)
                || m.source_frame > frames
                || i > 0
                    && (m.source_frame <= self.markers[i - 1].source_frame
                        || m.beat <= self.markers[i - 1].beat)
            {
                return Err("warp markers must increase in source and beat time".into());
            }
        }
        Ok(())
    }
}
impl Binding {
    pub fn verify(
        &self,
        source: &Path,
        wav: &Path,
        t: &Timeline,
        rate: u32,
    ) -> Result<(), ProjectError> {
        let r = hound::WavReader::open(source)?;
        self.recipe.validate(r.duration() as u64, t, rate)?;
        let map = TempoMap::compile(t)?;
        let start = map.sample_at(self.recipe.markers[0].beat, rate as f64)?;
        let end = map.sample_at(self.recipe.markers.last().unwrap().beat, rate as f64)?;
        let cache = hound::WavReader::open(wav)?;
        if self.schema != 1
            || !valid_id(&self.source)
            || self.sample_rate != rate
            || self.start_frame != start
            || self.frames != end - start
            || cache.duration() as u64 != self.frames
            || cache.spec().sample_rate != rate
            || cache.spec().channels != r.spec().channels
            || hash(source)? != self.source_sha256
            || hash(wav)? != self.wav_sha256
            || timeline_hash(t)? != self.timeline_sha256
        {
            return Err("stale or corrupt tempo cache; rebuild before publication".into());
        }
        Ok(())
    }
}
struct Reader {
    wav: hound::WavReader<std::io::BufReader<std::fs::File>>,
    spec: hound::WavSpec,
    pcm: Vec<f32>,
}
impl Reader {
    fn block(
        &mut self,
        source: [u64; 2],
        converted: u64,
        at: u64,
        out: &mut [f32],
    ) -> Result<(), ProjectError> {
        let ch = self.spec.channels as usize;
        let increment = (source[1] - source[0]) as f64 / converted as f64;
        if !(1. / 32. ..=32.).contains(&increment) {
            return Err("sample conversion ratio exceeds 1/32..32".into());
        }
        let frames = out.len() / ch;
        let first = source[0] as f64 + at as f64 * increment;
        let radius = ResampleQuality::SincFast.radius(increment) as u64;
        let begin = (first.floor() as u64).saturating_sub(radius).max(source[0]);
        let end =
            ((first + (frames.saturating_sub(1)) as f64 * increment).ceil() as u64 + radius + 1)
                .min(source[1]);
        let count = (end - begin) as usize;
        if count > READ {
            return Err("conversion working set exceeded".into());
        }
        self.wav.seek(begin as u32)?;
        if self.spec.sample_format == hound::SampleFormat::Float {
            let mut it = self.wav.samples::<f32>();
            for x in &mut self.pcm[..count * ch] {
                *x = it.next().ok_or("short source read")??;
            }
        } else {
            let scale = 2f64.powi(self.spec.bits_per_sample as i32 - 1) as f32;
            let mut it = self.wav.samples::<i32>();
            for x in &mut self.pcm[..count * ch] {
                *x = it.next().ok_or("short source read")?? as f32 / scale;
            }
        }
        if self.pcm[..count * ch]
            .iter()
            .any(|x| !x.is_finite() || x.abs() > 1e6)
        {
            return Err("invalid source samples".into());
        }
        for (i, frame) in out.chunks_exact_mut(ch).enumerate() {
            for (c, x) in frame.iter_mut().enumerate() {
                *x = ResampleQuality::SincFast.interpolate(
                    first + i as f64 * increment,
                    increment,
                    |p| {
                        if p >= begin as isize && (p as u64) < end {
                            self.pcm[(p as u64 - begin) as usize * ch + c]
                        } else {
                            0.
                        }
                    },
                );
            }
        }
        Ok(())
    }
}
/// Exclusively create `directory`; cancellation and errors remove only that newly created folder.
/// No method in this module belongs on a device callback.
pub fn prepare(
    source: &Path,
    source_id: &str,
    recipe: &Recipe,
    timeline: &Timeline,
    rate: u32,
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<Binding, ProjectError> {
    prepare_internal(
        source, source_id, recipe, timeline, rate, directory, cancel, None, 2,
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_linked(
    source: &Path,
    recipe: &Recipe,
    timeline: &Timeline,
    rate: u32,
    directory: &Path,
    cancel: &AtomicBool,
    reference: Option<usize>,
) -> Result<Binding, ProjectError> {
    prepare_internal(
        source, "linked", recipe, timeline, rate, directory, cancel, reference, 64,
    )
}
#[allow(clippy::too_many_arguments)]
fn prepare_internal(
    source: &Path,
    source_id: &str,
    recipe: &Recipe,
    timeline: &Timeline,
    rate: u32,
    directory: &Path,
    cancel: &AtomicBool,
    reference: Option<usize>,
    max_channels: u16,
) -> Result<Binding, ProjectError> {
    if !valid_id(source_id) {
        return Err("invalid source identity".into());
    }
    let wav = hound::WavReader::open(source)?;
    let spec = wav.spec();
    recipe.validate(wav.duration() as u64, timeline, rate)?;
    if !(1..=max_channels).contains(&spec.channels) || !(8000..=192000).contains(&spec.sample_rate)
    {
        return Err("warp source must be mono/stereo WAV at 8..192 kHz".into());
    }
    let map = TempoMap::compile(timeline)?;
    let start = map.sample_at(recipe.markers[0].beat, rate as f64)?;
    let end = map.sample_at(recipe.markers.last().unwrap().beat, rate as f64)?;
    if end <= start || end - start > u32::MAX as u64 / 2 || end - start > rate as u64 * 14400 {
        return Err("tempo render exceeds four-hour/RIFF bound".into());
    }
    let before = hash(source)?;
    std::fs::create_dir(directory)?;
    let result = (|| -> Result<Binding, ProjectError> {
        let mut reader = Reader {
            wav,
            spec,
            pcm: vec![0.; READ * spec.channels as usize],
        };
        let mut writer = hound::WavWriter::create(
            directory.join("audio.wav"),
            hound::WavSpec {
                sample_rate: rate,
                channels: spec.channels,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        let ch = spec.channels as usize;
        let mut input = vec![0.; BLOCK * ch];
        let mut output = input.clone();
        let mut segments = 0;
        for pair in recipe.markers.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            let mut beats = vec![a.beat, b.beat];
            for p in &timeline.tempo {
                if p.beat > a.beat && p.beat < b.beat {
                    beats.push(p.beat);
                }
            }
            // Subdivide linear ramps into at most half-beat average-rate sections.
            let ramp = timeline
                .tempo
                .iter()
                .any(|p| p.transition == soniclaw_transport::Transition::Linear && p.beat < b.beat);
            if ramp {
                let n = ((b.beat - a.beat) * 2.).ceil() as usize;
                if n > 8192 {
                    return Err("warp ramp needs too many sections".into());
                }
                for i in 1..n {
                    beats.push(a.beat + (b.beat - a.beat) * i as f64 / n as f64);
                }
            }
            beats.sort_by(f64::total_cmp);
            beats.dedup();
            segments += beats.len() - 1;
            if segments > 8192 {
                return Err("warp section bound exceeded".into());
            }
            for bs in beats.windows(2) {
                if cancel.load(Ordering::Acquire) {
                    return Err("tempo preparation cancelled".into());
                }
                let sf = |beat: f64| {
                    a.source_frame
                        + ((b.source_frame - a.source_frame) as f64 * (beat - a.beat)
                            / (b.beat - a.beat))
                            .round() as u64
                };
                let source = [sf(bs[0]), sf(bs[1])];
                let target =
                    map.sample_at(bs[1], rate as f64)? - map.sample_at(bs[0], rate as f64)?;
                if source[0] >= source[1] || target < 2 {
                    return Err("warp sections must contain at least two frames".into());
                }
                let converted = if recipe.mode == Mode::Repitch {
                    target
                } else {
                    ((source[1] - source[0]) as f64 * rate as f64 / spec.sample_rate as f64)
                        .round()
                        .max(1.) as u64
                };
                let ratio = target as f64 / converted as f64;
                let mut stretch = TimeStretcher::new(StretchSettings {
                    channels: ch,
                    ratio,
                    window_frames: 512,
                    search_frames: 64,
                    max_block_frames: BLOCK,
                    transient_ratio: Some(4.),
                    ..Default::default()
                })?;
                stretch.set_reference_channel(reference)?;
                let (mut consumed, mut written) = (0, 0u64);
                let mut last = vec![0.; ch];
                let mut emit = |samples: &[f32]| -> Result<(), ProjectError> {
                    for f in samples.chunks_exact(ch) {
                        if written >= target {
                            break;
                        }
                        let fade = (written.min(target - 1 - written) as f32 / 16.).min(1.);
                        for (c, &s) in f.iter().enumerate() {
                            writer.write_sample(s * fade)?;
                            last[c] = s;
                        }
                        written += 1;
                    }
                    Ok(())
                };
                while consumed < converted {
                    if cancel.load(Ordering::Acquire) {
                        return Err("tempo preparation cancelled".into());
                    }
                    let n = ((converted - consumed) as usize)
                        .min(BLOCK)
                        .min(stretch.input_space());
                    if n > 0 {
                        reader.block(source, converted, consumed, &mut input[..n * ch])?;
                        let accepted = stretch.push(&input[..n * ch])?;
                        consumed += accepted as u64;
                    }
                    let made = stretch.pull(&mut output, 8)?;
                    emit(&output[..made * ch])?;
                    if n == 0 && made == 0 {
                        return Err("stretcher made no progress".into());
                    }
                }
                stretch.finish();
                while !stretch.is_drained() {
                    let n = stretch.pull(&mut output, 8)?;
                    if n == 0 {
                        return Err("stretcher failed to drain".into());
                    }
                    emit(&output[..n * ch])?;
                }

                if written + 1 == target {
                    for x in &last[..ch] {
                        writer.write_sample(*x)?;
                    }
                    written += 1;
                }
                if written != target {
                    return Err("stretch duration mismatch".into());
                }
            }
        }
        writer.finalize()?;
        if cancel.load(Ordering::Acquire) || hash(source)? != before {
            return Err("cancelled or source changed during preparation".into());
        }
        let binding = Binding {
            schema: 1,
            source: source_id.into(),
            recipe: recipe.clone(),
            source_sha256: before,
            timeline_sha256: timeline_hash(timeline)?,
            wav_sha256: hash(&directory.join("audio.wav"))?,
            sample_rate: rate,
            start_frame: start,
            frames: end - start,
        };
        std::fs::write(
            directory.join("warp.json"),
            serde_json::to_vec_pretty(&binding)?,
        )?;
        Ok(binding)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result
}
impl Project {
    /// Verify and publish a prepared region and its portable cache metadata atomically.
    pub fn apply_warp(
        &mut self,
        track: &str,
        region: &str,
        asset: &str,
        binding: Binding,
        wav: &Path,
    ) -> Result<(), ProjectError> {
        if self
            .layout
            .elastic_groups
            .iter()
            .any(|g| g.members.iter().any(|m| m.asset == asset))
        {
            return Err(
                "rebuild the shared elastic group or dissolve it before independent edits".into(),
            );
        }
        if !valid_id(region)
            || !valid_id(asset)
            || asset == binding.source
            || (self.disk.contains_key(asset) && !self.warps.contains_key(asset))
            || (self.warps.len() >= 32 && !self.warps.contains_key(asset))
        {
            return Err("warp identity/capacity collision".into());
        }
        binding.verify(
            self.disk
                .get(&binding.source)
                .ok_or("missing original source")?,
            wav,
            &self.document.timeline,
            self.document.sample_rate,
        )?;
        if self.document.tracks.iter().any(|t| {
            t.streamed
                .iter()
                .any(|c| c.asset == asset && (t.id != track || c.id != region))
        }) {
            return Err("cache is shared; rebuild each dependent region explicitly".into());
        }
        let replacing = self.warps.contains_key(asset);
        let mut next = self.document.clone();
        let t = next
            .tracks
            .iter_mut()
            .find(|t| t.id == track)
            .ok_or("missing warp track")?;
        if replacing {
            t.streamed.retain(|c| !(c.id == region && c.asset == asset));
        }
        if t.audio.iter().any(|c| c.id == region) || t.streamed.iter().any(|c| c.id == region) {
            return Err("region already exists".into());
        }
        t.streamed.push(DiskClip {
            edits: None,
            id: region.into(),
            asset: asset.into(),
            start_frame: binding.start_frame,
            length_frames: binding.frames,
            source_offset: 0,
            source_length: binding.frames,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
        });
        next.validate()?;
        self.document = next;
        self.disk.insert(asset.into(), wav.into());
        self.warps.insert(asset.into(), binding);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_rate_warp_exact_clock_pitch_portability_stale_and_cancel() {
        let root = std::env::temp_dir().join(format!("soniclaw-warp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.wav");
        let mut w = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 44100,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..44100 {
            w.write_sample((i as f64 * 440. * std::f64::consts::TAU / 44100.).sin() as f32 * 0.2)
                .unwrap();
        }
        w.finalize().unwrap();
        let original = std::fs::read(&source).unwrap();
        let recipe = Recipe {
            schema: 1,
            mode: Mode::PreservePitch,
            markers: vec![
                Marker {
                    source_frame: 0,
                    beat: 0.,
                },
                Marker {
                    source_frame: 44100,
                    beat: 2.,
                },
            ],
        };
        let (mut d, a) = crate::tests::fixture();
        d.tracks[0].audio.clear();
        d.timeline.tempo = vec![soniclaw_transport::TempoPoint {
            beat: 0.,
            bpm: 90.,
            transition: soniclaw_transport::Transition::Step,
        }];
        let cache = root.join("cache");
        let b = prepare(
            &source,
            "original",
            &recipe,
            &d.timeline,
            48000,
            &cache,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(b.frames, 64000);
        let pcm = hound::WavReader::open(cache.join("audio.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(pcm.len(), 64000);
        let mid = &pcm[4096..pcm.len() - 4096];
        let crossings = mid.windows(2).filter(|p| p[0] <= 0. && p[1] > 0.).count();
        let hz = crossings as f64 * 48000. / mid.len() as f64;
        assert!((hz - 440.).abs() < 5., "{hz}");
        let mut p = Project::new(
            "Warp".into(),
            d,
            a,
            DiskAssets::from([("original".into(), source.clone())]),
        )
        .unwrap();
        p.apply_warp(
            "audio",
            "warped",
            "cache",
            b.clone(),
            &cache.join("audio.wav"),
        )
        .unwrap();
        p.save_new(&root.join("portable")).unwrap();
        let loaded = Project::open(&root.join("portable")).unwrap();
        assert_eq!(loaded.warps.len(), 1);
        let mut changed = loaded.document.timeline.clone();
        changed.tempo[0].bpm = 100.;
        assert!(b
            .verify(&source, &cache.join("audio.wav"), &changed, 48000)
            .is_err());
        let cancelled = root.join("cancelled");
        assert!(prepare(
            &source,
            "original",
            &recipe,
            &changed,
            48000,
            &cancelled,
            &AtomicBool::new(true)
        )
        .is_err());
        assert!(!cancelled.exists());
        // Exact anchors across a step change and a ramp. No accumulation of rounded durations.
        let mut ramp = changed.clone();
        ramp.tempo[0].bpm = 120.;
        ramp.tempo[0].transition = soniclaw_transport::Transition::Linear;
        ramp.tempo.push(soniclaw_transport::TempoPoint {
            beat: 1.,
            bpm: 90.,
            transition: soniclaw_transport::Transition::Step,
        });
        let rb = prepare(
            &source,
            "original",
            &recipe,
            &ramp,
            48000,
            &root.join("ramp"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            rb.frames,
            TempoMap::compile(&ramp)
                .unwrap()
                .sample_at(2., 48000.)
                .unwrap()
        );
        assert_eq!(original, std::fs::read(&source).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
