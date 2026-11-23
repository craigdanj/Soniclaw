//! Immutable loop passes and nondestructive comp recipes. Files are read/written only off callback.
use crate::multitrack::{BoundaryKind, RecordError, Take};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, path::Path};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    pub id: u32,
    pub start_sample: u64,
    pub end_sample: u64,
    pub source_frame: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub start: u64,
    pub end: u64,
    pub pass: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema: u32,
    pub take_id: String,
    pub loop_start: u64,
    pub loop_frames: u64,
    pub passes: Vec<Pass>,
    pub sections: Vec<Section>,
    pub crossfade_frames: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Region {
    pub file: String,
    pub timeline_frame: u64,
    pub source_offset: u64,
    pub frames: u64,
    pub fade_in: u64,
    pub fade_out: u64,
}
fn passes(take: &Take) -> Result<Vec<Pass>, RecordError> {
    if !take.complete
        || take.missing_frames != 0
        || take.queue_drops != 0
        || take.input_discontinuities != 0
    {
        return Err("comping requires complete gap-free take audio".into());
    }
    let end = take
        .file_start
        .checked_add(take.frames)
        .ok_or("take range overflow")?;
    let latency = take
        .tracks
        .iter()
        .map(|t| t.latency_frames as u64)
        .max()
        .unwrap_or(0);
    let mut result = Vec::new();
    for (id, b) in take.loop_boundaries.windows(2).enumerate() {
        if b[0].kind == BoundaryKind::Loop
            && b[1].kind == BoundaryKind::Loop
            && b[0].source_frame == b[1].source_frame
            && b[0].sample < b[1].sample
            && b[1].sample.checked_add(latency).is_some_and(|v| v <= end)
        {
            result.push(Pass {
                id: id as u32,
                start_sample: b[0].sample,
                end_sample: b[1].sample,
                source_frame: b[0].source_frame,
            });
        }
    }
    if result.is_empty() {
        return Err("no complete latency-covered loop passes".into());
    }
    Ok(result)
}
impl Recipe {
    pub fn new(take: &Take) -> Result<Self, RecordError> {
        let passes = passes(take)?;
        let first = &passes[0];
        let recipe = Self {
            schema: 1,
            take_id: take.id.clone(),
            loop_start: first.source_frame,
            loop_frames: first.end_sample - first.start_sample,
            sections: vec![Section {
                start: 0,
                end: first.end_sample - first.start_sample,
                pass: first.id,
            }],
            passes,
            crossfade_frames: 0,
        };
        recipe.validate(take)?;
        Ok(recipe)
    }
    pub fn validate(&self, take: &Take) -> Result<(), RecordError> {
        if self.schema != 1
            || self.take_id != take.id
            || self.passes != passes(take)?
            || self.sections.is_empty()
            || self.sections.len() > 256
            || self.crossfade_frames == 1
            || self.crossfade_frames > 4096
        {
            return Err("invalid comp recipe".into());
        }
        if self.loop_frames == 0
            || self.passes.iter().any(|p| {
                p.source_frame != self.loop_start
                    || p.end_sample - p.start_sample != self.loop_frames
            })
        {
            return Err("loop length/tempo changed; split the recording before comping".into());
        }
        let mut at = 0;
        for section in &self.sections {
            if section.start != at
                || section.end <= section.start
                || section.end > self.loop_frames
                || !self.passes.iter().any(|p| p.id == section.pass)
                || section.end - section.start < self.crossfade_frames.saturating_mul(2)
            {
                return Err("comp sections must cover one loop, with room for fades".into());
            }
            at = section.end;
        }
        if at != self.loop_frames {
            return Err("comp does not cover complete loop".into());
        }
        Ok(())
    }
    /// Replace the selected frame range; split/merge recipe sections transactionally.
    pub fn choose(
        &mut self,
        take: &Take,
        start: u64,
        end: u64,
        pass: u32,
    ) -> Result<(), RecordError> {
        self.validate(take)?;
        if start >= end || end > self.loop_frames || !self.passes.iter().any(|p| p.id == pass) {
            return Err("invalid comp selection".into());
        }
        let mut sections = Vec::new();
        for s in &self.sections {
            if s.end <= start || s.start >= end {
                sections.push(s.clone());
                continue;
            }
            if s.start < start {
                sections.push(Section {
                    start: s.start,
                    end: start,
                    pass: s.pass,
                });
            }
            sections.push(Section {
                start: s.start.max(start),
                end: s.end.min(end),
                pass,
            });
            if s.end > end {
                sections.push(Section {
                    start: end,
                    end: s.end,
                    pass: s.pass,
                });
            }
        }
        let mut merged: Vec<Section> = Vec::new();
        for section in sections {
            if let Some(last) = merged.last_mut() {
                if last.pass == section.pass && last.end == section.start {
                    last.end = section.end;
                    continue;
                }
            }
            merged.push(section);
        }
        let mut candidate = self.clone();
        candidate.sections = merged;
        candidate.validate(take)?;
        *self = candidate;
        Ok(())
    }
    pub fn to_json(&self, take: &Take) -> Result<String, RecordError> {
        self.validate(take)?;
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(text: &str, take: &Take) -> Result<Self, RecordError> {
        if text.len() > 1024 * 1024 {
            return Err("comp recipe too large".into());
        }
        let recipe: Self = serde_json::from_str(text)?;
        recipe.validate(take)?;
        Ok(recipe)
    }
    /// Source offsets include each stem's recorded processing latency exactly once.
    pub fn regions(&self, take: &Take, track: &str) -> Result<Vec<Region>, RecordError> {
        self.validate(take)?;
        let track = take
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or("recorded stem missing")?;
        let mut regions = Vec::new();
        for (index, section) in self.sections.iter().enumerate() {
            let pass = self
                .passes
                .iter()
                .find(|p| p.id == section.pass)
                .ok_or("pass missing")?;
            let left = if index > 0 {
                self.crossfade_frames / 2
            } else {
                0
            };
            let right = if index + 1 < self.sections.len() {
                self.crossfade_frames - self.crossfade_frames / 2
            } else {
                0
            };
            let relative = section.start.checked_sub(left).ok_or("fade before pass")?;
            let offset = pass
                .start_sample
                .checked_add(track.latency_frames as u64)
                .and_then(|v| v.checked_sub(take.file_start))
                .and_then(|v| v.checked_add(relative))
                .ok_or("comp source range overflow")?;
            let frames = section.end - section.start + left + right;
            if offset.checked_add(frames).is_none_or(|v| v > take.frames) {
                return Err("comp extends beyond recorded stem".into());
            }
            regions.push(Region {
                file: track.file.clone(),
                timeline_frame: self.loop_start + relative,
                source_offset: offset,
                frames,
                fade_in: if left > 0 {
                    self.crossfade_frames - 1
                } else {
                    0
                },
                fade_out: if right > 0 {
                    self.crossfade_frames - 1
                } else {
                    0
                },
            });
        }
        Ok(regions)
    }
    /// Stream an immutable comp to a new float32 WAV using a fixed 1024-frame mix buffer.
    pub fn render_new(
        &self,
        directory: &Path,
        track: &str,
        output: &Path,
    ) -> Result<(), RecordError> {
        let take = Take::read(directory)?;
        let regions = self.regions(&take, track)?;
        let stem = take
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or("stem missing")?;
        let mut reader = hound::WavReader::open(directory.join(&stem.file))?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        let result = (|| -> Result<(), RecordError> {
            let mut writer = hound::WavWriter::new(file, reader.spec())?;
            let mut pcm = vec![0.; 1024 * stem.channels];
            let mut at = 0;
            while at < self.loop_frames {
                let frames = (self.loop_frames - at).min(1024) as usize;
                pcm[..frames * stem.channels].fill(0.);
                for r in &regions {
                    let start = (r.timeline_frame - self.loop_start).max(at);
                    let end =
                        (r.timeline_frame - self.loop_start + r.frames).min(at + frames as u64);
                    if start >= end {
                        continue;
                    }
                    let local = start - (r.timeline_frame - self.loop_start);
                    reader.seek(u32::try_from(r.source_offset + local)?)?;
                    for frame in start..end {
                        let position = frame - (r.timeline_frame - self.loop_start);
                        let mut gain = 1f32;
                        if r.fade_in > 0 {
                            gain *= position.min(r.fade_in) as f32 / r.fade_in as f32;
                        }
                        if r.fade_out > 0 {
                            gain *= (r.frames - 1 - position).min(r.fade_out) as f32
                                / r.fade_out as f32;
                        }
                        for channel in 0..stem.channels {
                            let sample = reader
                                .samples::<f32>()
                                .next()
                                .ok_or("truncated comp source")??;
                            pcm[(frame - at) as usize * stem.channels + channel] += sample * gain;
                        }
                    }
                }
                for sample in &pcm[..frames * stem.channels] {
                    writer.write_sample(*sample)?;
                }
                at += frames as u64;
            }
            writer.finalize()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(output);
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::multitrack::*;
    #[test]
    fn marked_passes_comp_crossfade_and_immutable_sources() {
        let dir = std::env::temp_dir().join(format!("comp54-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let (mut recorder, mut tap) = pair(Config {
            sample_rate: 48000,
            tracks: vec![TrackSpec {
                id: "stem".into(),
                channels: 1,
                latency_frames: 0,
            }],
            pre_roll_frames: 0,
            queue_packets: 32,
            max_block_frames: 64,
        })
        .unwrap();
        recorder
            .start(
                dir.join("take"),
                TakePlan {
                    id: "take".into(),
                    label: "passes".into(),
                    punch: None,
                },
            )
            .unwrap();
        let data: Vec<_> = (0..64).map(|i| [0.2, 0.4, 0.8, 0.1][i / 16]).collect();
        let boundaries: Vec<_> = [0, 16, 32, 48]
            .into_iter()
            .map(|sample| LoopBoundary {
                sample,
                source_frame: 100,
                kind: BoundaryKind::Loop,
            })
            .collect();
        let measured = allocation_counter::measure(|| {
            tap.capture_with_loops(0, &[&data], &boundaries).unwrap()
        });
        assert_eq!(measured.count_total, 0);
        let take = recorder.stop().unwrap();
        assert_eq!(take.loop_boundaries.len(), 4);
        let original = std::fs::read(dir.join("take/stem.wav")).unwrap();
        let mut recipe = Recipe::new(&take).unwrap();
        recipe.choose(&take, 8, 16, 2).unwrap();
        recipe.crossfade_frames = 2;
        let json = recipe.to_json(&take).unwrap();
        let restored = Recipe::from_json(&json, &take).unwrap();
        restored
            .render_new(&dir.join("take"), "stem", &dir.join("comp.wav"))
            .unwrap();
        let rendered = hound::WavReader::open(dir.join("comp.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rendered.len(), 16);
        assert_eq!(rendered[0], 0.2);
        assert_eq!(rendered[15], 0.8);
        assert_eq!(std::fs::read(dir.join("take/stem.wav")).unwrap(), original);
        let before = recipe.to_json(&take).unwrap();
        assert!(recipe.choose(&take, 2, 3, 999).is_err());
        assert_eq!(recipe.to_json(&take).unwrap(), before);
        assert!(recipe
            .render_new(&dir.join("take"), "stem", &dir.join("comp.wav"))
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
