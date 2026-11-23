//! Nondestructive source/curve metadata and transactional audio region edits.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FadeShape {
    #[default]
    Linear,
    EqualPower,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GainPoint {
    pub frame: u64,
    pub gain: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionEdits {
    pub schema: u32,
    pub source_phase: u64,
    pub fade_offset: u64,
    pub fade_length: u64,
    pub envelope_offset: u64,
    pub envelope_length: u64,
    pub envelope: Vec<GainPoint>,
    pub fade_shape: FadeShape,
    pub link_group: Option<String>,
}
impl RegionEdits {
    pub fn new(length: u64) -> Self {
        Self {
            schema: 1,
            source_phase: 0,
            fade_offset: 0,
            fade_length: length,
            envelope_offset: 0,
            envelope_length: length,
            envelope: vec![],
            fade_shape: FadeShape::Linear,
            link_group: None,
        }
    }
    pub fn validate(&self, length: u64, fade_in: u64, fade_out: u64) -> Result<(), Error> {
        if self.schema != 1
            || self.fade_length == 0
            || self.envelope_length == 0
            || self.fade_length > MAX_FRAME
            || self.envelope_length > MAX_FRAME
            || [self.source_phase, self.fade_offset, self.envelope_offset]
                .iter()
                .any(|v| v.checked_add(length).is_none_or(|n| n > MAX_FRAME))
            || fade_in
                .checked_add(fade_out)
                .is_none_or(|n| n > self.fade_length)
            || self.envelope.len() > 1024
            || self.envelope.windows(2).any(|w| w[0].frame >= w[1].frame)
            || self.envelope.iter().any(|p| {
                p.frame > self.envelope_length
                    || !p.gain.is_finite()
                    || !(0.0..=8.).contains(&p.gain)
            })
            || self.link_group.as_ref().is_some_and(|id| !valid_id(id))
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    pub fn envelope_value(&self, local: u64) -> f32 {
        let frame = self.envelope_offset.saturating_add(local);
        let index = self.envelope.partition_point(|p| p.frame <= frame);
        if index == 0 {
            return self.envelope.first().map_or(1., |p| p.gain);
        }
        if index == self.envelope.len() {
            return self.envelope[index - 1].gain;
        }
        let a = &self.envelope[index - 1];
        let b = &self.envelope[index];
        let t = (frame - a.frame) as f64 / (b.frame - a.frame) as f64;
        (a.gain as f64 + (b.gain - a.gain) as f64 * t) as f32
    }
    pub fn gain_at(
        metadata: Option<&Self>,
        local: u64,
        length: u64,
        fade_in: u64,
        fade_out: u64,
        base: f32,
    ) -> f32 {
        let (mut position, mut span, mut shape) = (local, length, FadeShape::Linear);
        let mut gain = base;
        if let Some(m) = metadata {
            gain *= m.envelope_value(local);
            position = m.fade_offset.saturating_add(local);
            span = m.fade_length;
            shape = m.fade_shape;
        }
        let curve = |v: f32| {
            if shape == FadeShape::EqualPower {
                (v * std::f32::consts::FRAC_PI_2).sin()
            } else {
                v
            }
        };
        if fade_in > 0 {
            gain *= curve(position.min(fade_in) as f32 / fade_in as f32);
        }
        if fade_out > 0 {
            gain *= curve(
                span.saturating_sub(1)
                    .saturating_sub(position)
                    .min(fade_out) as f32
                    / fade_out as f32,
            );
        }
        gain
    }
}
impl AudioClip {
    pub fn source_phase(&self) -> u64 {
        self.edits.as_ref().map_or(0, |m| m.source_phase)
    }
    pub fn region_gain(&self, local: u64) -> f32 {
        RegionEdits::gain_at(
            self.edits.as_deref(),
            local,
            self.length_frames,
            self.fade_in,
            self.fade_out,
            self.gain,
        )
    }
    pub fn edit_metadata(&mut self) -> &mut RegionEdits {
        let length = self.length_frames;
        self.edits
            .get_or_insert_with(|| Box::new(RegionEdits::new(length)))
    }
}
fn index(d: &Document, id: &str) -> Result<usize, Error> {
    d.audio
        .iter()
        .position(|c| c.id == id)
        .ok_or(Error::InvalidParameter)
}
pub(crate) fn apply_extra(d: &mut Document, edit: &Edit) -> Result<bool, Error> {
    match edit {
        Edit::SplitAudio { id, new_id, frame } => {
            let i = index(d, id)?;
            let old = d.audio[i].clone();
            let at = frame
                .checked_sub(old.start_frame)
                .filter(|n| *n > 0 && *n < old.length_frames)
                .ok_or(Error::InvalidParameter)?;
            if !valid_id(new_id)
                || d.audio.iter().any(|c| c.id == *new_id)
                || d.midi.iter().any(|c| c.id == *new_id)
            {
                return Err(Error::InvalidParameter);
            }
            let mut left = old.clone();
            let mut right = old;
            left.edit_metadata();
            right.edit_metadata();
            left.length_frames = at;
            right.id = new_id.clone();
            right.start_frame = *frame;
            right.length_frames -= at;
            let m = right.edits.as_deref_mut().expect("created");
            m.source_phase = m
                .source_phase
                .checked_add(at)
                .ok_or(Error::InvalidParameter)?;
            m.fade_offset += at;
            m.envelope_offset += at;
            d.audio[i] = left;
            d.audio.insert(i + 1, right);
        }
        Edit::JoinAudio { left, right } => {
            let a = index(d, left)?;
            let b = index(d, right)?;
            if a == b {
                return Err(Error::InvalidParameter);
            }
            let mut l = d.audio[a].clone();
            let r = d.audio[b].clone();
            if l.start_frame + l.length_frames != r.start_frame
                || l.asset != r.asset
                || l.track != r.track
                || l.gain != r.gain
                || l.looping != r.looping
                || l.enabled != r.enabled
            {
                return Err(Error::InvalidParameter);
            }
            let lm = l
                .edits
                .as_deref()
                .cloned()
                .unwrap_or_else(|| RegionEdits::new(l.length_frames));
            let rm = r
                .edits
                .as_deref()
                .cloned()
                .unwrap_or_else(|| RegionEdits::new(r.length_frames));
            let same_curves = lm.fade_length == rm.fade_length
                && lm.fade_offset + l.length_frames == rm.fade_offset
                && lm.envelope_length == rm.envelope_length
                && lm.envelope_offset + l.length_frames == rm.envelope_offset
                && lm.envelope == rm.envelope
                && lm.fade_shape == rm.fade_shape
                && lm.link_group == rm.link_group
                && l.fade_in == r.fade_in
                && l.fade_out == r.fade_out;
            if l.source_offset == r.source_offset
                && l.source_length == r.source_length
                && lm.source_phase + l.length_frames == rm.source_phase
                && same_curves
            {
                l.length_frames += r.length_frames;
            } else if !l.looping
                && lm.envelope.is_empty()
                && rm.envelope.is_empty()
                && l.fade_out == 0
                && r.fade_in == 0
                && lm.fade_offset == 0
                && rm.fade_offset == 0
                && lm.fade_length == l.length_frames
                && rm.fade_length == r.length_frames
                && lm.fade_shape == rm.fade_shape
                && lm.link_group == rm.link_group
                && lm.source_phase + l.length_frames <= l.source_length
                && rm.source_phase + r.length_frames <= r.source_length
                && l.source_offset + lm.source_phase + l.length_frames
                    == r.source_offset + rm.source_phase
            {
                l.source_offset += lm.source_phase;
                l.length_frames += r.length_frames;
                l.source_length = l.length_frames;
                l.fade_out = r.fade_out;
                let mut m = RegionEdits::new(l.length_frames);
                m.fade_shape = lm.fade_shape;
                m.link_group = lm.link_group;
                l.edits = Some(Box::new(m));
            } else {
                return Err(Error::InvalidParameter);
            }
            d.audio[a] = l;
            d.audio.remove(b);
        }
        Edit::SlipAudio { id, frames } => {
            let i = index(d, id)?;
            let looping = d.audio[i].looping;
            let source_length = d.audio[i].source_length;
            let m = d.audio[i].edit_metadata();
            m.source_phase = if looping {
                (m.source_phase as i128 + *frames as i128).rem_euclid(source_length as i128) as u64
            } else {
                m.source_phase
                    .checked_add_signed(*frames)
                    .ok_or(Error::InvalidParameter)?
            };
        }
        Edit::GainEnvelope { id, points } => {
            if points.len() > 1024 {
                return Err(Error::InvalidParameter);
            }
            let i = index(d, id)?;
            let length = d.audio[i].length_frames;
            let m = d.audio[i].edit_metadata();
            m.envelope_offset = 0;
            m.envelope_length = length;
            m.envelope = points.clone();
        }
        Edit::LinkAudio { ids, group } => {
            if ids.is_empty()
                || ids.len() > 256
                || ids.iter().collect::<BTreeSet<_>>().len() != ids.len()
                || group.as_ref().is_some_and(|id| !valid_id(id))
            {
                return Err(Error::InvalidParameter);
            }
            for id in ids {
                let i = index(d, id)?;
                d.audio[i].edit_metadata().link_group = group.clone();
            }
        }
        Edit::CrossfadeAudio { left, right, shape } => {
            let a = index(d, left)?;
            let b = index(d, right)?;
            if a == b {
                return Err(Error::InvalidParameter);
            }
            let l = &d.audio[a];
            let r = &d.audio[b];
            let end = l.start_frame + l.length_frames;
            if l.track != r.track
                || l.start_frame >= r.start_frame
                || r.start_frame >= end
                || end > r.start_frame + r.length_frames
                || end - r.start_frame < 2
            {
                return Err(Error::InvalidParameter);
            }
            let overlap = end - r.start_frame;
            d.audio[a].fade_out = overlap - 1;
            d.audio[b].fade_in = overlap - 1;
            for i in [a, b] {
                let length = d.audio[i].length_frames;
                let m = d.audio[i].edit_metadata();
                m.fade_offset = 0;
                m.fade_length = length;
                m.fade_shape = *shape;
            }
        }
        Edit::MoveAudio {
            id,
            frame,
            snap_frames,
        } => {
            let i = index(d, id)?;
            let target = if *snap_frames == 0 {
                *frame
            } else {
                u64::try_from(
                    (*frame as u128 + *snap_frames as u128 / 2) / *snap_frames as u128
                        * *snap_frames as u128,
                )
                .map_err(|_| Error::InvalidParameter)?
            };
            let delta = target as i128 - d.audio[i].start_frame as i128;
            let group = d.audio[i].edits.as_ref().and_then(|m| m.link_group.clone());
            for (index, c) in d.audio.iter_mut().enumerate() {
                if index == i
                    || group.as_ref().is_some_and(|g| {
                        c.edits.as_ref().and_then(|m| m.link_group.as_ref()) == Some(g)
                    })
                {
                    c.start_frame = u64::try_from(c.start_frame as i128 + delta)
                        .map_err(|_| Error::InvalidParameter)?;
                }
            }
        }
        Edit::RemoveClip { id } => {
            let Some(c) = d.audio.iter().find(|c| c.id == *id) else {
                return Ok(false);
            };
            let Some(group) = c.edits.as_ref().and_then(|m| m.link_group.clone()) else {
                return Ok(false);
            };
            d.audio
                .retain(|c| c.edits.as_ref().and_then(|m| m.link_group.as_ref()) != Some(&group));
        }
        _ => return Ok(false),
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> (Document, Assets) {
        let clip = AudioClip {
            edits: None,
            id: "audio".into(),
            asset: "samples".into(),
            track: 0,
            start_frame: 0,
            length_frames: 64,
            source_offset: 0,
            source_length: 8,
            looping: true,
            gain: 0.7,
            fade_in: 7,
            fade_out: 11,
            enabled: true,
        };
        (
            Document {
                schema: 1,
                sample_rate: 48000,
                channels: 1,
                timeline: Default::default(),
                audio: vec![clip],
                midi: vec![],
            },
            Assets::from([(
                "samples".into(),
                AudioAsset {
                    sample_rate: 48000,
                    channels: 1,
                    samples: Arc::from([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]),
                },
            )]),
        )
    }
    fn render(d: &Document, a: &Assets, frames: usize) -> Vec<f32> {
        let mut p = d.compile(a, 17, None).unwrap();
        let mut out = vec![0.; frames];
        let mut at = 0;
        let measured = allocation_counter::measure(|| {
            for block in out.chunks_mut(17) {
                p.render(
                    &mut AudioBuffer::new(block, 1).unwrap(),
                    Transport {
                        sample_position: at,
                        tempo: 120.,
                        playing: true,
                    },
                )
                .unwrap();
                at += block.len() as u64;
            }
        });
        assert_eq!(measured.count_total, 0);
        out
    }
    #[test]
    fn split_join_preserves_loop_phase_and_original_curves_exactly() {
        let (mut d, a) = document();
        d.audio[0].edit_metadata().fade_shape = FadeShape::EqualPower;
        d.apply(Edit::GainEnvelope {
            id: "audio".into(),
            points: vec![
                GainPoint {
                    frame: 0,
                    gain: 0.2,
                },
                GainPoint {
                    frame: 31,
                    gain: 0.9,
                },
                GainPoint {
                    frame: 64,
                    gain: 0.4,
                },
            ],
        })
        .unwrap();
        let original = d.to_json().unwrap();
        let pcm = render(&d, &a, 64);
        d.apply(Edit::SplitAudio {
            id: "audio".into(),
            new_id: "right".into(),
            frame: 23,
        })
        .unwrap();
        assert_eq!(render(&d, &a, 64), pcm);
        d.apply(Edit::JoinAudio {
            left: "audio".into(),
            right: "right".into(),
        })
        .unwrap();
        assert_eq!(d.to_json().unwrap(), original);
        assert_eq!(render(&d, &a, 64), pcm);
        let before = d.to_json().unwrap();
        assert!(d
            .apply(Edit::SplitAudio {
                id: "audio".into(),
                new_id: "bad".into(),
                frame: 0
            })
            .is_err());
        assert_eq!(d.to_json().unwrap(), before);
        d.apply(Edit::SlipAudio {
            id: "audio".into(),
            frames: -1,
        })
        .unwrap();
        assert_eq!(d.audio[0].source_phase(), 7);
    }
    #[test]
    fn overlap_crossfade_and_linked_operations_are_nondestructive() {
        let (mut d, mut a) = document();
        a.insert(
            "samples".into(),
            AudioAsset {
                sample_rate: 48000,
                channels: 1,
                samples: Arc::from([1.; 64]),
            },
        );
        d.audio[0].length_frames = 16;
        d.audio[0].source_length = 16;
        d.audio[0].looping = false;
        d.audio[0].gain = 1.;
        d.audio[0].fade_in = 0;
        d.audio[0].fade_out = 0;
        let mut right = d.audio[0].clone();
        right.id = "right".into();
        right.start_frame = 8;
        d.audio.push(right);
        d.apply(Edit::CrossfadeAudio {
            left: "audio".into(),
            right: "right".into(),
            shape: FadeShape::Linear,
        })
        .unwrap();
        assert!(render(&d, &a, 24).iter().all(|s| (*s - 1.).abs() < 1e-6));
        d.apply(Edit::LinkAudio {
            ids: vec!["audio".into(), "right".into()],
            group: Some("pair".into()),
        })
        .unwrap();
        d.apply(Edit::MoveAudio {
            id: "audio".into(),
            frame: 16,
            snap_frames: 0,
        })
        .unwrap();
        assert_eq!(
            d.audio.iter().map(|c| c.start_frame).collect::<Vec<_>>(),
            vec![16, 24]
        );
        let old = d.apply(Edit::RemoveClip { id: "audio".into() }).unwrap();
        assert!(d.audio.is_empty());
        d = old;
        d.apply(Edit::GainEnvelope {
            id: "audio".into(),
            points: vec![GainPoint {
                frame: 0,
                gain: 0.5,
            }],
        })
        .unwrap();
        assert_eq!(a["samples"].samples.as_ref(), &[1.; 64]);
        let json = d.to_json().unwrap();
        assert_eq!(Document::from_json(&json).unwrap().audio.len(), 2);
    }
}
