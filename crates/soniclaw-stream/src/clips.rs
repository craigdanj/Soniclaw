//! Bounded disk-backed arrangement regions, using existing seekable page workers.
use crate::{open_seekable_wav, DiskHandle, DiskStats, SeekablePlayer};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
pub type DiskAssets = BTreeMap<String, PathBuf>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskClip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<Box<soniclaw_clips::RegionEdits>>,
    pub id: String,
    pub asset: String,
    pub start_frame: u64,
    pub length_frames: u64,
    pub source_offset: u64,
    pub source_length: u64,
    pub looping: bool,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    pub enabled: bool,
}
impl DiskClip {
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(m) = &self.edits {
            m.validate(self.length_frames, self.fade_in, self.fade_out)?;
        }

        let id = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if !id(&self.id)
            || !id(&self.asset)
            || self.length_frames == 0
            || self.source_length == 0
            || self
                .start_frame
                .checked_add(self.length_frames)
                .is_none_or(|v| v > 1 << 48)
            || self
                .source_offset
                .checked_add(self.source_length)
                .is_none_or(|v| v > u32::MAX as u64)
            || self
                .fade_in
                .checked_add(self.fade_out)
                .is_none_or(|v| v > self.length_frames)
            || !self.gain.is_finite()
            || self.gain.abs() > 4.
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct StreamTelemetry {
    pub clip_id: String,
    pub cache_bytes: usize,
    pub stats: Arc<DiskStats>,
}
struct Source {
    clip: DiskClip,
    player: SeekablePlayer,
    handle: DiskHandle,
    last: Option<u64>,
}
pub struct StreamedClips {
    sources: Vec<Source>,
    scratch: Vec<f32>,
    spec: ProcessSpec,
}
impl StreamedClips {
    pub fn new(
        clips: &[DiskClip],
        assets: &DiskAssets,
        spec: ProcessSpec,
        cache_bytes_per_clip: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        spec.validate()?;
        if spec.channels != 2
            || spec.max_block_frames > 1024
            || clips.len() > 16
            || !(128 * 1024..=2 * 1024 * 1024).contains(&cache_bytes_per_clip)
        {
            return Err(Error::InvalidSpec.into());
        }
        let mut ids = BTreeSet::new();
        for c in clips {
            c.validate()?;
            if !ids.insert(&c.id) {
                return Err(Error::InvalidParameter.into());
            }
        }
        let mut sources = Vec::new();
        for c in clips.iter().filter(|c| c.enabled) {
            let path = assets.get(&c.asset).ok_or(Error::InvalidParameter)?;
            let reader = hound::WavReader::open(path)?;
            if reader.spec().sample_rate as f64 != spec.sample_rate
                || reader.spec().channels > 2
                || c.source_offset + c.source_length > reader.duration() as u64
            {
                return Err(Error::InvalidSpec.into());
            }
            drop(reader);
            let (handle, mut player) = open_seekable_wav(path, cache_bytes_per_clip)?;
            player.prepare(spec)?;
            player.set_loop(if c.looping {
                Some((c.source_offset, c.source_offset + c.source_length))
            } else {
                None
            })?;
            player.seek(c.source_offset)?;
            player.set_playing(false);
            sources.push(Source {
                clip: c.clone(),
                player,
                handle,
                last: None,
            });
        }
        Ok(Self {
            sources,
            scratch: vec![0.; spec.max_block_frames * 2],
            spec,
        })
    }
    pub fn telemetry(&self) -> Vec<StreamTelemetry> {
        self.sources
            .iter()
            .map(|s| StreamTelemetry {
                clip_id: s.clip.id.clone(),
                cache_bytes: s.handle.pcm_cache_bytes,
                stats: s.handle.stats.clone(),
            })
            .collect()
    }
    /// Control-thread prefetch before publishing/after stopping. No guarantees of future disk throughput.
    pub fn prime_at(&mut self, sample: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut ready = true;
        for s in &mut self.sources {
            let local = sample.saturating_sub(s.clip.start_frame)
                + s.clip.edits.as_ref().map_or(0, |m| m.source_phase);
            if !s.clip.looping && local >= s.clip.source_length {
                continue;
            }
            let source = s.clip.source_offset
                + if s.clip.looping {
                    local % s.clip.source_length
                } else {
                    local.min(s.clip.source_length)
                };
            if s.player.position() != source && s.player.seek(source).is_err() {
                ready = false;
                continue;
            }
            s.player.set_playing(false);
            loop {
                s.player.process(
                    &mut AudioBuffer::new(&mut [], 2).expect("stereo"),
                    &ProcessContext {
                        events: &[],
                        transport: Default::default(),
                    },
                );
                if s.player.ready_frames(self.spec.max_block_frames + 1)
                    > self.spec.max_block_frames
                {
                    break;
                }
                s.player.prefetch();
                let remain = deadline.saturating_duration_since(Instant::now());
                std::thread::sleep(Duration::from_millis(1).min(remain));
                if remain.is_zero() || !s.handle.wait_ready(1, remain) {
                    ready = false;
                    break;
                }
            }
        }
        ready
    }
    /// Control/offline thread only. Preflight interpolation lookahead and reject every observed disk miss.
    pub fn mix_offline_into(
        &mut self,
        output: &mut [f32],
        transport: Transport,
    ) -> Result<(), Error> {
        if !self.prime_at(transport.sample_position, Duration::from_secs(2)) {
            return Err(Error::NotPrepared);
        }
        self.mix_into(output, transport)?;
        if self.sources.iter().any(|s| {
            s.handle
                .stats
                .underrun_frames
                .load(std::sync::atomic::Ordering::Acquire)
                > 0
                || s.handle
                    .stats
                    .io_errors
                    .load(std::sync::atomic::Ordering::Acquire)
                    > 0
        }) {
            return Err(Error::NotPrepared);
        }
        Ok(())
    }
    /// Add streamed regions to existing stereo PCM. Disk misses are silence and visible in telemetry.
    pub fn mix_into(&mut self, output: &mut [f32], transport: Transport) -> Result<(), Error> {
        if !output.len().is_multiple_of(2)
            || output.len() / 2 > self.spec.max_block_frames
            || transport
                .sample_position
                .checked_add((output.len() / 2) as u64)
                .is_none()
        {
            return Err(Error::InvalidBuffer);
        }
        let frames = output.len() / 2;
        for s in &mut self.sources {
            s.player.set_playing(false);
            s.player.process(
                &mut AudioBuffer::new(&mut [], 2)?,
                &ProcessContext {
                    events: &[],
                    transport,
                },
            );
            s.player.prefetch();
            if !transport.playing {
                continue;
            }
            let clip_end = s.clip.start_frame + s.clip.length_frames;
            let start = transport.sample_position.max(s.clip.start_frame);
            let end = (transport.sample_position + frames as u64).min(clip_end);
            if start >= end {
                continue;
            }
            let local = start - s.clip.start_frame;
            let phase = local + s.clip.edits.as_ref().map_or(0, |m| m.source_phase);
            if !s.clip.looping && phase >= s.clip.source_length {
                continue;
            }
            let source = s.clip.source_offset
                + if s.clip.looping {
                    phase % s.clip.source_length
                } else {
                    phase
                };
            if s.player.position() != source {
                s.player.seek(source)?;
            }
            let n = (end - start) as usize;
            let n = if s.clip.looping {
                n
            } else {
                n.min((s.clip.source_length - phase) as usize)
            };
            let scratch = &mut self.scratch[..n * 2];
            s.player.set_playing(true);
            s.player.process(
                &mut AudioBuffer::new(scratch, 2)?,
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: start,
                        ..transport
                    },
                },
            );
            let offset = (start - transport.sample_position) as usize;
            for i in 0..n {
                let position = local + i as u64;
                let gain = soniclaw_clips::RegionEdits::gain_at(
                    s.clip.edits.as_deref(),
                    position,
                    s.clip.length_frames,
                    s.clip.fade_in,
                    s.clip.fade_out,
                    s.clip.gain,
                );
                for ch in 0..2 {
                    output[(offset + i) * 2 + ch] += scratch[i * 2 + ch] * gain;
                }
            }
            s.last = Some(end);
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        for s in &mut self.sources {
            s.player.set_playing(false);
            s.last = None;
        }
    }
}
impl AudioProcessor for StreamedClips {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.channels != 2
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        b.clear();
        if b.channels() != 2 || self.mix_into(b.samples_mut(), ctx.transport).is_err() {
            b.clear();
        }
    }
    fn reset(&mut self) {
        self.reset();
    }
}
impl DiskClip {
    /// Control-side model conversion; no decoding or PCM copying.
    pub fn as_audio(&self, track: u16) -> soniclaw_clips::AudioClip {
        soniclaw_clips::AudioClip {
            edits: self.edits.clone(),
            id: self.id.clone(),
            asset: self.asset.clone(),
            track,
            start_frame: self.start_frame,
            length_frames: self.length_frames,
            source_offset: self.source_offset,
            source_length: self.source_length,
            looping: self.looping,
            gain: self.gain,
            fade_in: self.fade_in,
            fade_out: self.fade_out,
            enabled: self.enabled,
        }
    }
    pub fn from_audio(c: soniclaw_clips::AudioClip) -> Self {
        Self {
            edits: c.edits,
            id: c.id,
            asset: c.asset,
            start_frame: c.start_frame,
            length_frames: c.length_frames,
            source_offset: c.source_offset,
            source_length: c.source_length,
            looping: c.looping,
            gain: c.gain,
            fade_in: c.fade_in,
            fade_out: c.fade_out,
            enabled: c.enabled,
        }
    }
}
/// Apply the shared nondestructive edit semantics to disk regions transactionally.
pub fn edit_regions(
    regions: &mut Vec<DiskClip>,
    edit: soniclaw_clips::Edit,
    sample_rate: u32,
) -> Result<(), Error> {
    for c in regions.iter() {
        c.validate()?;
    }
    let mut doc = soniclaw_clips::Document {
        schema: 1,
        sample_rate,
        channels: 2,
        timeline: Default::default(),
        audio: regions.iter().map(|c| c.as_audio(0)).collect(),
        midi: vec![],
    };
    doc.apply(edit)?;
    let next = doc
        .audio
        .into_iter()
        .map(DiskClip::from_audio)
        .collect::<Vec<_>>();
    for c in &next {
        c.validate()?;
    }
    *regions = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, DiskAssets, DiskClip) {
        let path = std::env::temp_dir().join(format!(
            "streamed-clips-{}-{}.wav",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                sample_rate: 48000,
                channels: 1,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..12000 {
            w.write_sample(if i < 4096 { 0.25f32 } else { -0.5 })
                .unwrap();
        }
        w.finalize().unwrap();
        (
            path.clone(),
            DiskAssets::from([("a".into(), path)]),
            DiskClip {
                edits: None,
                id: "a".into(),
                asset: "a".into(),
                start_frame: 10,
                length_frames: 100,
                source_offset: 0,
                source_length: 64,
                looping: true,
                gain: 1.,
                fade_in: 10,
                fade_out: 10,
                enabled: true,
            },
        )
    }
    #[test]
    fn offsets_fades_loops_overlap_and_no_callback_allocations() {
        let (path, assets, clip) = fixture();
        let spec = ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 256,
        };
        let mut second = clip.clone();
        second.id = "b".into();
        second.gain = 0.5;
        let mut set = StreamedClips::new(&[clip, second], &assets, spec, 1024 * 1024).unwrap();
        assert!(set.prime_at(0, Duration::from_secs(1)));
        let mut out = [0.; 256];
        let alloc =
            allocation_counter::measure(|| set.mix_into(&mut out, Transport::default()).unwrap());
        assert_eq!(alloc.count_total, 0);
        assert_eq!(out[0], 0.);
        assert_eq!(out[20], 0.);
        assert!((out[40] - 0.375).abs() < 1e-6);
        assert_eq!(out[218], 0.);
        assert_eq!(out[220], 0.);
        drop(set);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn seek_prefetch_generation_and_metadata_rejection() {
        let (path, assets, mut clip) = fixture();
        clip.start_frame = 0;
        clip.length_frames = 12000;
        clip.source_length = 12000;
        clip.looping = false;
        clip.fade_in = 0;
        clip.fade_out = 0;
        let spec = ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 256,
        };
        let mut set =
            StreamedClips::new(std::slice::from_ref(&clip), &assets, spec, 1024 * 1024).unwrap();
        assert!(set.prime_at(6000, Duration::from_secs(1)));
        let mut out = [0.; 256];
        set.mix_into(
            &mut out,
            Transport {
                sample_position: 6000,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.iter().all(|v| (*v + 0.5).abs() < 1e-6));
        assert!(set.prime_at(0, Duration::from_secs(1)));
        out.fill(0.);
        set.mix_into(&mut out, Transport::default()).unwrap();
        assert!(out.iter().all(|v| (*v - 0.25).abs() < 1e-6));
        assert!(set.telemetry()[0].cache_bytes < 12000 * 8);
        assert!(StreamedClips::new(
            &[clip],
            &assets,
            ProcessSpec {
                sample_rate: 44100.,
                ..spec
            },
            1024 * 1024
        )
        .is_err());
        drop(set);
        std::fs::remove_file(path).unwrap();
    }
}
